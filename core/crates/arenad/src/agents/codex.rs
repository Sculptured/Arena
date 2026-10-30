//! Codex CLI driver (`codex exec`), signed in with the user's subscription.
//!
//! Flags (confirmed live against codex 0.163.0):
//!   exec --skip-git-repo-check --json -s read-only -C <dir>
//!   -c 'mcp_servers.retail.url="<url>"'
//!   -c 'mcp_servers.retail.default_tools_approval_mode="approve"' -c 'approval_policy="never"'
//!   [-m <model>] <prompt>
//! `--ignore-user-config` keeps ~/.codex/config.toml (other MCP servers, plugins, hooks) out;
//! auth still comes from CODEX_HOME. `approval_policy=never` alone makes every MCP call fail ("requires approval, but approval
//! policy is never"); the per-server `default_tools_approval_mode="approve"` fixes that.
//! Codex's built-in shell cannot be disabled; `-s read-only` is the only restriction on it.

use std::{future::Future, path::Path, pin::Pin, process::Command};

use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, CLI_TIMEOUT_SECS};
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

pub fn command(spec: &AgentSpec, mcp_url: &str, workdir: &Path, prompt: &str) -> Command {
    command_with("codex", spec, mcp_url, workdir, prompt)
}

fn command_with(
    binary: &str,
    spec: &AgentSpec,
    mcp_url: &str,
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
        .arg(format!("mcp_servers.retail.url={}", json!(mcp_url)))
        .args([
            "-c",
            "mcp_servers.retail.default_tools_approval_mode=\"approve\"",
        ])
        .args(["-c", "approval_policy=\"never\""]);
    if let Some(m) = &spec.model {
        c.args(["-m", m]);
    }
    c.arg(prompt);
    super::scrub_env(&mut c);
    c
}

/// Parse `--json` JSONL stdout. `seconds` is left 0 for the caller. `cost_usd` is None
/// (subscription usage has no per-call price).
pub fn parse(stdout: &str, returncode: i32) -> AgentResult {
    let (mut prompt, mut completion) = (0i64, 0i64);
    let (mut completed, mut error) = (false, None::<String>);
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("turn.completed") => {
                completed = true;
                prompt += v["usage"]["input_tokens"].as_i64().unwrap_or(0);
                completion += v["usage"]["output_tokens"].as_i64().unwrap_or(0);
            }
            Some("turn.failed") => {
                error = Some(
                    v["error"]["message"]
                        .as_str()
                        .unwrap_or("turn failed")
                        .to_string(),
                );
            }
            Some("error") => {
                error = Some(v["message"].as_str().unwrap_or("codex error").to_string());
            }
            _ => {}
        }
    }
    let ok = returncode == 0 && completed;
    let error = if ok {
        None
    } else {
        Some(error.unwrap_or_else(|| {
            if returncode != 0 {
                format!("codex exited with code {returncode}")
            } else {
                "codex produced no completed turn".into()
            }
        }))
    };
    AgentResult {
        ok,
        error,
        tokens: json!({"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion}),
        cost_usd: None,
        cost_source: None,
        seconds: 0.0,
    }
}

impl Driver for CodexDriver {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            if run.resume.is_some() {
                return super::resume_unsupported();
            }
            let workdir = match super::fresh_workdir() {
                Ok(d) => d,
                Err(e) => return super::fail(format!("cannot create workdir: {e}")),
            };
            let cmd = command_with(
                &self.binary,
                &run.spec,
                &super::mcp_url(&self.self_base, &run),
                &workdir,
                &super::prompt_of(&run),
            );
            let r = super::run_cli(cmd, self.timeout_secs, parse).await;
            let _ = std::fs::remove_dir_all(&workdir);
            r
        })
    }
}
