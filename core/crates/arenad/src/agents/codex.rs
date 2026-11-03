//! Codex CLI driver (`codex exec`), signed in with the user's subscription.
//!
//! Flags (confirmed live against codex 0.163.0):
//!   exec --skip-git-repo-check --json -s read-only -C <dir>
//!   -c 'mcp_servers.<server>.url="<url>"'
//!   -c 'mcp_servers.<server>.default_tools_approval_mode="approve"' -c 'approval_policy="never"'
//!   [-m <model>] <prompt>
//! `<server>` is the benchmark's name (`super::server_name`).
//! `--ignore-user-config` keeps ~/.codex/config.toml (other MCP servers, plugins, hooks) out;
//! auth still comes from CODEX_HOME. `approval_policy=never` alone makes every MCP call fail ("requires approval, but approval
//! policy is never"); the per-server `default_tools_approval_mode="approve"` fixes that.
//! Codex's built-in shell cannot be disabled; `-s read-only` is the only restriction on it.

use std::{future::Future, path::Path, pin::Pin, process::Command};

use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, Mcp, CLI_TIMEOUT_SECS};
use crate::model::AgentSpec;

pub struct CodexDriver {
    pub self_base: String,
    pub binary: String,
    pub timeout_secs: u64,
}

impl CodexDriver {
    pub fn new(self_base: impl Into<String>) -> CodexDriver {
        CodexDriver {
            self_base: self_base.into(),
            binary: "codex".into(),
            timeout_secs: CLI_TIMEOUT_SECS,
        }
    }
}

const DISABLED_FEATURES: &[&str] = &[
    "apps",
    "plugins",
    "remote_plugin",
    "hooks",
    "image_generation",
    "multi_agent",
    "goals",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "tool_suggest",
    "skill_mcp_dependency_install",
    "skill_search",
];

pub fn command(spec: &AgentSpec, mcp: &Mcp, workdir: &Path, prompt: &str) -> Command {
    command_with("codex", spec, mcp, workdir, prompt)
}

fn command_with(
    binary: &str,
    spec: &AgentSpec,
    mcp: &Mcp,
    workdir: &Path,
    prompt: &str,
) -> Command {
    let mut c = Command::new(binary);
    c.args(["exec", "--skip-git-repo-check", "--json", "-s", "read-only"])
        .args(["--ignore-user-config", "--ignore-rules", "--ephemeral"]);
    // Account-level extras that --ignore-user-config does not remove (ChatGPT apps/connectors,
    // plugins, hooks, browser/computer use, image generation, sub-agents, goals).
    for f in DISABLED_FEATURES {
        c.args(["--disable", f]);
    }
    c.args(["-c", "web_search=\"disabled\""])
        .arg("-C")
        .arg(workdir)
        .arg("-c")
        .arg(format!("mcp_servers.{}.url={}", mcp.server, json!(mcp.url)))
        .arg("-c")
        .arg(format!(
            "mcp_servers.{}.default_tools_approval_mode=\"approve\"",
            mcp.server
        ))
        .args(["-c", "approval_policy=\"never\""]);
    if let Some(m) = &spec.model {
        c.args(["-m", m]);
    }
    // The prompt is positional and may start with "-": keep it apart from the flags.
    c.args(["--", prompt]);
    super::scrub_env(&mut c, "OPENAI_API_KEY");
    c
}

/// What the JSONL events of a run add up to.
#[derive(Default)]
struct Turns {
    prompt: i64,
    completion: i64,
    completed: bool,
    error: Option<String>,
}

fn scan_events(stdout: &str) -> Turns {
    let mut t = Turns::default();
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("turn.completed") => {
                t.completed = true;
                t.prompt += v["usage"]["input_tokens"].as_i64().unwrap_or(0);
                t.completion += v["usage"]["output_tokens"].as_i64().unwrap_or(0);
            }
            Some("turn.failed") => {
                let m = v["error"]["message"].as_str().unwrap_or("turn failed");
                t.error = Some(m.to_string());
            }
            Some("error") => {
                t.error = Some(v["message"].as_str().unwrap_or("codex error").to_string());
            }
            _ => {}
        }
    }
    t
}

/// Parse `--json` JSONL stdout. `seconds` is left 0 for the caller. `cost_usd` is None
/// (subscription usage has no per-call price).
pub fn parse(stdout: &str, returncode: i32) -> AgentResult {
    let t = scan_events(stdout);
    let ok = returncode == 0 && t.completed;
    let error = (!ok).then(|| {
        t.error.unwrap_or_else(|| {
            if returncode != 0 {
                format!("codex exited with code {returncode}")
            } else {
                "codex produced no completed turn".into()
            }
        })
    });
    AgentResult {
        ok,
        error,
        tokens: super::tokens_json(t.prompt, t.completion, t.prompt + t.completion),
        cost_usd: None,
        cost_source: None,
        seconds: 0.0,
        failure: None,
    }
}

impl Driver for CodexDriver {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let mcp = Mcp::of(&self.self_base, &run);
            let prompt = super::prompt_of(&run);
            let build =
                |workdir: &Path| command_with(&self.binary, &run.spec, &mcp, workdir, &prompt);
            super::run_in_workdir(&run, self.timeout_secs, build, parse).await
        })
    }
}
