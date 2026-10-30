use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::Value;

use crate::{exec::BranchCtx, model::AgentSpec};

pub mod claude;
pub mod codex;
pub mod open;

/// One agent's turn in a branch.
pub struct AgentRun {
    /// Shared with the MCP endpoint so both use the same per-branch gate.
    pub branch: Arc<BranchCtx>,
    pub spec: AgentSpec,
    /// The first user message.
    pub input: String,
    /// A replayed message list (system message is replaced by `spec.prompt`).
    pub resume: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentResult {
    pub ok: bool,
    pub error: Option<String>,
    /// `{prompt_tokens, completion_tokens, total_tokens}` summed over model calls.
    pub tokens: Value,
    /// Summed cost; None if any call lacked a cost.
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub seconds: f64,
}

pub trait Driver: Send + Sync {
    fn run<'a>(&'a self, ctx: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>>;
}

pub const CLI_TIMEOUT_SECS: u64 = 900;
const STDERR_TAIL: usize = 2048;

/// Last `STDERR_TAIL` bytes of `s`, cut on a char boundary.
pub(crate) fn tail(s: &str) -> &str {
    if s.len() <= STDERR_TAIL {
        return s;
    }
    let mut i = s.len() - STDERR_TAIL;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

fn failed(error: String, seconds: f64) -> AgentResult {
    AgentResult {
        ok: false,
        error: Some(error),
        tokens: serde_json::json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}),
        cost_usd: None,
        cost_source: None,
        seconds,
    }
}

/// Spawn `cmd` (stdin null, output captured, killed on drop) and wait up to `timeout_secs`.
/// `parse` maps (stdout, exit code) to a result; `seconds` is filled in here.
pub(crate) async fn run_cli(
    cmd: std::process::Command,
    timeout_secs: u64,
    parse: impl FnOnce(&str, i32) -> AgentResult,
) -> AgentResult {
    let start = std::time::Instant::now();
    let mut cmd = tokio::process::Command::from(cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return failed(format!("failed to start CLI: {e}"), 0.0),
    };
    let waited = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await;
    let seconds = start.elapsed().as_secs_f64();
    match waited {
        Err(_) => failed(format!("timed out after {timeout_secs} s"), seconds),
        Ok(Err(e)) => failed(format!("CLI failed: {e}"), seconds),
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let code = out.status.code().unwrap_or(-1);
            let mut r = parse(&stdout, code);
            r.seconds = seconds;
            if !r.ok && !stderr.trim().is_empty() {
                let msg = r
                    .error
                    .take()
                    .unwrap_or_else(|| format!("exit code {code}"));
                r.error = Some(format!("{msg}\nstderr: {}", tail(stderr.trim())));
            }
            r
        }
    }
}

pub(crate) fn resume_unsupported() -> AgentResult {
    failed("CLI agents cannot resume mid-run".to_string(), 0.0)
}

pub(crate) fn prompt_of(run: &AgentRun) -> String {
    format!("{}\n\n{}", run.spec.prompt, run.input)
}

pub(crate) fn mcp_url(self_base: &str, run: &AgentRun) -> String {
    format!(
        "{}/mcp/{}/{}",
        self_base.trim_end_matches('/'),
        run.branch.branch_id,
        run.spec.id
    )
}

pub(crate) fn fresh_workdir() -> std::io::Result<std::path::PathBuf> {
    let d = std::env::temp_dir().join(format!("arena-cli-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub(crate) fn fail(error: String) -> AgentResult {
    failed(error, 0.0)
}

/// Remove database/API secrets from a CLI child's environment.
pub(crate) fn scrub_env(c: &mut std::process::Command) {
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy();
        if name.starts_with("PG")
            || matches!(
                name.as_ref(),
                "ARENA_PG_URL" | "DATABASE_URL" | "OPENROUTER_API_KEY"
            )
        {
            c.env_remove(&k);
        }
    }
    for k in [
        "ARENA_PG_URL",
        "DATABASE_URL",
        "PGPASSWORD",
        "OPENROUTER_API_KEY",
    ] {
        c.env_remove(k);
    }
}
