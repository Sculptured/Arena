//! Claude Code CLI driver (`claude -p`), signed in with the user's subscription.
//!
//! Flags (confirmed live against claude 2.1.314):
//!   -p <prompt> --model <m> --output-format json --setting-sources ""
//!   --strict-mcp-config --mcp-config '{"mcpServers":{"<server>":{"type":"http","url":..}}}'
//!   --disable-slash-commands --tools "" --allowedTools "mcp__<server>__*"
//! `<server>` is the benchmark's name (`super::server_name`).
//! `--tools ""` removes the built-in tools; `--allowedTools` pre-approves the MCP tools so
//! no permission prompt blocks the non-interactive run. A dispatch run's session takes
//! `--input-format stream-json --output-format stream-json --verbose` instead of `--output-format
//! json` and the prompt (confirmed live against claude 2.1.319).

use std::{future::Future, pin::Pin, process::Command};

use serde_json::{json, Value};

use super::{
    session::{Session, SessionStart},
    AgentResult, AgentRun, Driver, Mcp, CLI_TIMEOUT_SECS,
};
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

pub fn command(spec: &AgentSpec, mcp: &Mcp, prompt: &str) -> Command {
    command_with("claude", spec, mcp, prompt)
}

fn command_with(binary: &str, spec: &AgentSpec, mcp: &Mcp, prompt: &str) -> Command {
    let mut c = isolated(binary, spec, mcp);
    // The prompt is positional and may start with "-": keep it apart from the flags.
    c.args(["--output-format", "json"]).args(["--", prompt]);
    c
}

/// Claude Code headless with only the world's MCP tools, and none of the user's settings,
/// sessions, slash commands or built-in tools; its environment scrubbed of other secrets.
fn isolated(binary: &str, spec: &AgentSpec, mcp: &Mcp) -> Command {
    let config = json!({"mcpServers": {&mcp.server: {"type": "http", "url": mcp.url}}});
    let mut c = Command::new(binary);
    c.arg("-p")
        .arg("--model")
        .arg(spec.model.as_deref().unwrap_or("sonnet"))
        .args(["--setting-sources", ""])
        .arg("--strict-mcp-config")
        .arg("--mcp-config")
        .arg(config.to_string())
        .arg("--disable-slash-commands")
        .args(["--tools", ""])
        .arg("--allowedTools")
        .arg(format!("mcp__{}__*", mcp.server))
        .arg("--no-session-persistence");
    super::scrub_env(&mut c, "ANTHROPIC_API_KEY");
    c
}

/// A long-lived session (see `claude_session`): the isolated command taking its turns as a stream
/// of user messages on stdin and answering in stream-json.
pub fn session_command(binary: &str, spec: &AgentSpec, mcp: &Mcp) -> Command {
    let mut c = isolated(binary, spec, mcp);
    c.args(["--input-format", "stream-json"])
        .args(["--output-format", "stream-json"])
        .arg("--verbose");
    c
}

/// Parse `--output-format json` stdout. `seconds` is left 0 for the caller.
pub fn parse(stdout: &str) -> AgentResult {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return super::fail(format!(
            "unparseable claude output: {}",
            super::tail(stdout)
        ));
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
        tokens: super::tokens_json(prompt, completion, prompt + completion),
        cost_usd: v["total_cost_usd"].as_f64(),
        cost_source: Some("api_equivalent_estimate".into()),
        seconds: 0.0,
        failure: None,
    }
}

impl Driver for ClaudeDriver {
    fn session(&self, start: SessionStart) -> Result<Box<dyn Session>, String> {
        let s = super::claude_session::ClaudeSession::start(&self.self_base, &self.binary, start)?;
        Ok(Box::new(s))
    }

    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let mcp = Mcp::of(&self.self_base, &run);
            let prompt = super::prompt_of(&run);
            let build = |workdir: &std::path::Path| {
                let mut cmd = command_with(&self.binary, &run.spec, &mcp, &prompt);
                cmd.current_dir(workdir);
                cmd
            };
            super::run_in_workdir(&run, self.timeout_secs, build, |out, _| parse(out)).await
        })
    }
}
