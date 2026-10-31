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

/// Why an agent failed, when it is not an infrastructure problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The model's own behavior (too many calls, output cut off): the branch is `failed`.
    Model,
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
    /// `None` on failure means an infrastructure error (CLI or gateway/upstream problem).
    pub failure: Option<FailureKind>,
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

/// `{prompt_tokens, completion_tokens, total_tokens}`.
pub(crate) fn tokens_json(prompt: i64, completion: i64, total: i64) -> Value {
    serde_json::json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}

fn failed(error: String, seconds: f64) -> AgentResult {
    AgentResult {
        ok: false,
        error: Some(error),
        tokens: tokens_json(0, 0, 0),
        cost_usd: None,
        cost_source: None,
        seconds,
        failure: None,
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
        Ok(Ok(out)) => finish_cli(&out, seconds, parse),
    }
}

/// The result of a CLI that ran to completion; a failure carries the tail of its stderr.
fn finish_cli(
    out: &std::process::Output,
    seconds: f64,
    parse: impl FnOnce(&str, i32) -> AgentResult,
) -> AgentResult {
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

/// Runs a CLI agent in a fresh work folder (removed afterwards). `build` makes the command for
/// that folder; CLI agents can't resume.
pub(crate) async fn run_in_workdir(
    run: &AgentRun,
    timeout_secs: u64,
    build: impl FnOnce(&std::path::Path) -> std::process::Command,
    parse: impl FnOnce(&str, i32) -> AgentResult,
) -> AgentResult {
    if run.resume.is_some() {
        return resume_unsupported();
    }
    let workdir = match fresh_workdir() {
        Ok(d) => d,
        Err(e) => return fail(format!("cannot create workdir: {e}")),
    };
    let r = run_cli(build(&workdir), timeout_secs, parse).await;
    let _ = std::fs::remove_dir_all(&workdir);
    r
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
