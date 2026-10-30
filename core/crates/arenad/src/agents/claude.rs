//! Claude Code CLI driver (`claude -p`), signed in with the user's subscription.
//!
//! Flags (confirmed live against claude 2.1.314):
//!   -p <prompt> --model <m> --output-format json --setting-sources ""
//!   --strict-mcp-config --mcp-config '{"mcpServers":{"retail":{"type":"http","url":..}}}'
//!   --disable-slash-commands --tools "" --allowedTools "mcp__retail__*"
//! `--tools ""` removes the built-in tools; `--allowedTools` pre-approves the MCP tools so
//! no permission prompt blocks the non-interactive run.

use std::{future::Future, pin::Pin, process::Command};

use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, CLI_TIMEOUT_SECS};
use crate::model::AgentSpec;

pub struct ClaudeDriver {
    pub self_base: String,
    pub binary: String,
    pub timeout_secs: u64,
}

impl ClaudeDriver {
    pub fn new(self_base: impl Into<String>) -> ClaudeDriver {
        ClaudeDriver {
            self_base: self_base.into(),
            binary: "claude".into(),
            timeout_secs: CLI_TIMEOUT_SECS,
        }
    }
}

pub fn command(spec: &AgentSpec, mcp_url: &str, prompt: &str) -> Command {
    command_with("claude", spec, mcp_url, prompt)
}

fn command_with(binary: &str, spec: &AgentSpec, mcp_url: &str, prompt: &str) -> Command {
    let mcp = json!({"mcpServers": {"retail": {"type": "http", "url": mcp_url}}}).to_string();
    let mut c = Command::new(binary);
    c.arg("-p")
        .arg("--model")
        .arg(spec.model.as_deref().unwrap_or("sonnet"))
        .args(["--output-format", "json"])
        .args(["--setting-sources", ""])
        .arg("--strict-mcp-config")
        .arg("--mcp-config")
        .arg(mcp)
        .arg("--disable-slash-commands")
        .args(["--tools", ""])
        .args(["--allowedTools", "mcp__retail__*"])
        .arg("--no-session-persistence")
        // The prompt is positional and may start with "-": keep it apart from the flags.
        .args(["--", prompt]);
    super::scrub_env(&mut c);
    c
}

/// Parse `--output-format json` stdout. `seconds` is left 0 for the caller.
pub fn parse(stdout: &str) -> AgentResult {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return AgentResult {
            ok: false,
            error: Some(format!(
                "unparseable claude output: {}",
                super::tail(stdout)
            )),
            tokens: json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}),
            cost_usd: None,
            cost_source: None,
            seconds: 0.0,
            failure: None,
        };
    };
    let u = &v["usage"];
    let n = |k: &str| u[k].as_i64().unwrap_or(0);
    let prompt =
        n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    let completion = n("output_tokens");
    let is_error = v["is_error"].as_bool().unwrap_or(false);
    let text = v["result"].as_str().unwrap_or("").to_string();
    AgentResult {
        ok: !is_error,
        error: is_error.then(|| {
            if text.is_empty() {
                "claude reported an error".into()
            } else {
                text
            }
        }),
        tokens: json!({"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion}),
        cost_usd: v["total_cost_usd"].as_f64(),
        cost_source: Some("api_equivalent_estimate".into()),
        seconds: 0.0,
        failure: None,
    }
}

impl Driver for ClaudeDriver {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            if run.resume.is_some() {
                return super::resume_unsupported();
            }
            let workdir = match super::fresh_workdir() {
                Ok(d) => d,
                Err(e) => return super::fail(format!("cannot create workdir: {e}")),
            };
            let mut cmd = command_with(
                &self.binary,
                &run.spec,
                &super::mcp_url(&self.self_base, &run),
                &super::prompt_of(&run),
            );
            cmd.current_dir(&workdir);
            let r = super::run_cli(cmd, self.timeout_secs, |out, _| parse(out)).await;
            let _ = std::fs::remove_dir_all(&workdir);
            r
        })
    }
}
