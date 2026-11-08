//! Asking the reviewer model: Claude Code headless (`claude -p`, no tools, JSON out, the prompt on
//! stdin) or one of arenad's providers (one chat-completions call). Records what it cost.

use std::{process::Stdio, time::Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::{output::json_in, prompt::Prompt, settings::ReviewerModel};
use crate::{prices, providers::ProviderConfig};

/// How long a review may take.
pub const TIMEOUT_SECS: u64 = 600;
/// The `max_tokens` of a provider call.
const MAX_TOKENS: u64 = 6000;

/// The reviewer's answer: its JSON, and what it cost.
#[derive(Debug, Clone)]
pub struct Answer {
    pub value: Value,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub tokens: Option<i64>,
    pub seconds: f64,
}

pub async fn ask(
    model: &ReviewerModel,
    prompt: &Prompt,
    providers: &ProviderConfig,
    http: &reqwest::Client,
) -> Result<Answer> {
    let start = Instant::now();
    let mut a = match model {
        ReviewerModel::ClaudeCode { model } => claude(model, prompt).await?,
        ReviewerModel::Provider { provider, model } => {
            provider_call(provider, model, prompt, providers, http).await?
        }
    };
    a.seconds = start.elapsed().as_secs_f64();
    Ok(a)
}

/// The `claude -p` command: the given model, no tools, no MCP servers, no settings or session
/// kept, the system prompt replaced by the reviewer's, and the answer's schema.
pub fn claude_command(model: &str, prompt: &Prompt) -> std::process::Command {
    let mut c = std::process::Command::new("claude");
    c.arg("-p")
        .args(["--model", model])
        .args(["--output-format", "json"])
        .args(["--setting-sources", ""])
        .arg("--strict-mcp-config")
        .arg("--disable-slash-commands")
        .args(["--tools", ""])
        .arg("--no-session-persistence")
        .args(["--system-prompt", &prompt.system])
        .args(["--json-schema", &prompt.schema.to_string()]);
    crate::agents::scrub_env(&mut c, "ANTHROPIC_API_KEY");
    c
}

async fn claude(model: &str, prompt: &Prompt) -> Result<Answer> {
    let dir = crate::agents::fresh_workdir().context("making a work folder for claude")?;
    let out = run_claude(model, prompt, &dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    parse_claude(&out?)
}

async fn run_claude(model: &str, prompt: &Prompt, dir: &std::path::Path) -> Result<String> {
    let mut cmd = tokio::process::Command::from(claude_command(model, prompt));
    cmd.current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().context("starting claude")?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin.write_all(prompt.user.as_bytes()).await?;
    drop(stdin);
    let limit = std::time::Duration::from_secs(TIMEOUT_SECS);
    let out = tokio::time::timeout(limit, child.wait_with_output())
        .await
        .map_err(|_| anyhow!("the reviewer did not answer within {TIMEOUT_SECS} s"))??;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() && stdout.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("claude failed: {}", crate::agents::tail(err.trim()));
    }
    Ok(stdout)
}

/// `claude --output-format json` stdout: the structured answer (or the JSON in its text), the
/// cost it reports and its tokens.
pub fn parse_claude(stdout: &str) -> Result<Answer> {
    let v: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| anyhow!("unparseable claude output: {}", crate::agents::tail(stdout)))?;
    let text = v["result"].as_str().unwrap_or("");
    if v["is_error"].as_bool().unwrap_or(false) {
        bail!("claude reported an error: {}", super::text::cut(text, 500));
    }
    let value = match &v["structured_output"] {
        Value::Object(_) => v["structured_output"].clone(),
        _ => json_in(text)?,
    };
    let u = &v["usage"];
    let n = |k: &str| u[k].as_i64().unwrap_or(0);
    let tokens = n("input_tokens")
        + n("cache_creation_input_tokens")
        + n("cache_read_input_tokens")
        + n("output_tokens");
    Ok(Answer {
        value,
        cost_usd: v["total_cost_usd"].as_f64(),
        cost_source: Some("api_equivalent_estimate".into()),
        tokens: Some(tokens),
        seconds: 0.0,
    })
}

/// One chat-completions request: the reviewer's system prompt with its schema, and the digest.
fn chat_request(model: &str, prompt: &Prompt) -> Value {
    let system = format!(
        "{}\n\nThe JSON schema of your answer:\n{}",
        prompt.system, prompt.schema
    );
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt.user},
        ],
    })
}

async fn provider_call(
    provider: &str,
    model: &str,
    prompt: &Prompt,
    providers: &ProviderConfig,
    http: &reqwest::Client,
) -> Result<Answer> {
    let p = providers.resolve(Some(provider)).map_err(|e| anyhow!(e))?;
    let up = (p.send(http, &chat_request(model, prompt)).await)
        .map_err(|e| anyhow!("{provider}: {e}"))?;
    if !(200..300).contains(&up.status) {
        let msg = up.response["error"]["message"].as_str();
        let msg = super::text::cut(msg.unwrap_or("no message"), 300);
        bail!("{provider} answered {}: {msg}", up.status);
    }
    let text = up.response["choices"][0]["message"]["content"].as_str();
    let usage = &up.response["usage"];
    let (pt, ct) = (
        usage["prompt_tokens"].as_i64(),
        usage["completion_tokens"].as_i64(),
    );
    let (cost_usd, cost_source) = match (p.cost(usage), p.cost_source()) {
        (Some(c), Some(s)) => (Some(c), Some(s.to_string())),
        _ => list_price(&p.id, model, pt.unwrap_or(0), ct.unwrap_or(0)),
    };
    Ok(Answer {
        value: json_in(text.unwrap_or(""))?,
        cost_usd,
        cost_source,
        tokens: pt.zip(ct).map(|(a, b)| a + b),
        seconds: 0.0,
    })
}

/// The cost at the list price of `docs/prices.toml`, when it has the model.
fn list_price(
    provider: &str,
    model: &str,
    prompt: i64,
    completion: i64,
) -> (Option<f64>, Option<String>) {
    match prices::table().find(provider, model) {
        Some(p) => (
            Some(p.cost(prompt, completion)),
            Some("list-price estimate".into()),
        ),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claude_command_has_no_tools_and_keeps_only_its_own_key() {
        let p = super::super::prompt::run_prompt(&json!({}));
        let c = claude_command("sonnet", &p);
        let args: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].as_str())
        };
        assert_eq!(at("--model"), Some("sonnet"));
        assert_eq!(at("--tools"), Some(""));
        assert_eq!(at("--output-format"), Some("json"));
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert!(!args.contains(&"--mcp-config".to_string()));
        assert_eq!(at("--system-prompt"), Some(p.system.as_str()));
        // The digest goes on stdin, not on the command line.
        assert!(!args.iter().any(|a| a.contains("The digest of")));
        let removed: Vec<String> = c
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert!(removed.contains(&"OPENROUTER_API_KEY".to_string()));
        assert!(!removed.contains(&"ANTHROPIC_API_KEY".to_string()));
    }

    #[test]
    fn claude_output_gives_the_answer_and_its_cost() {
        // The shape of claude 2.1.319's `--output-format json` with `--json-schema`.
        let out = json!({"type": "result", "is_error": false, "result": "{\"verdict\":\"v\"}",
            "structured_output": {"verdict": "v"}, "total_cost_usd": 0.0123,
            "usage": {"input_tokens": 2, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 10, "output_tokens": 50}});
        let a = parse_claude(&out.to_string()).unwrap();
        assert_eq!(a.value["verdict"], "v");
        assert_eq!(a.cost_usd, Some(0.0123));
        assert_eq!(a.tokens, Some(162));
        // Without structured output, the JSON in the text.
        let out = json!({"is_error": false, "result": "```json\n{\"verdict\": \"w\"}\n```"});
        assert_eq!(
            parse_claude(&out.to_string()).unwrap().value["verdict"],
            "w"
        );
        let err = json!({"is_error": true, "result": "Credit balance is too low"});
        assert!(parse_claude(&err.to_string())
            .unwrap_err()
            .to_string()
            .contains("Credit"));
        assert!(parse_claude("not json").is_err());
    }
}
