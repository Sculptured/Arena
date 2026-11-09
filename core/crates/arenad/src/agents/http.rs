//! An agent deployed anywhere, behind a URL (`kind: "http"`), spoken to with `arena-agent/1`:
//!
//! - **Run.** arenad POSTs `{protocol, type: "run", run_id, branch_id, agent, task, mcp_url,
//!   token, deadline}` to the URL, with the agent's headers. The agent works through the MCP
//!   endpoint at `mcp_url` (each call with `Authorization: Bearer <token>`), where every tool call
//!   is recorded as for any agent, and answers the POST when it is done: 2xx with an optional
//!   `{ok, error, usage: {input_tokens, output_tokens}, cost_usd}`. The token stops working once
//!   arenad stops waiting.
//! - **Hello.** `POST {protocol, type: "hello"}`: any 2xx is a handshake, `{name}` optional.
//!
//! No answer by the deadline, or `ok: false`, is a failed run (the agent's doing); an agent that
//! can't be reached or answers an error status is an infrastructure error.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, FailureKind};
use crate::{model::AgentSpec, store::Store};

pub const PROTOCOL: &str = "arena-agent/1";
/// The longest a hello may take.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How an agent's own cost is labelled.
pub const COST_SOURCE: &str = "agent_reported";

pub struct HttpDriver {
    /// The arenad base URL the agent is told to reach the MCP endpoint at.
    pub agent_base: String,
    pub store: Arc<Store>,
    pub http: reqwest::Client,
    /// How long arenad waits for the agent's answer (time paused left out).
    pub timeout: Duration,
}

impl HttpDriver {
    pub fn new(agent_base: String, store: Arc<Store>, timeout: Duration) -> HttpDriver {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("http client");
        HttpDriver {
            agent_base,
            store,
            http,
            timeout,
        }
    }
}

/// The URL's host (and port), the only part of it an error names: a URL may carry a credential
/// in its query.
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| u.port().map_or(h.to_string(), |p| format!("{h}:{p}")))
        })
        .unwrap_or_else(|| "the agent".into())
}

/// Why `url` can't be an agent's address, if it can't.
pub fn bad_url(url: Option<&str>) -> Option<String> {
    let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else {
        return Some("an http agent needs a url".into());
    };
    match reqwest::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => None,
        _ => Some(format!("{url:?} is not an http(s) URL")),
    }
}

/// The agent's headers with their values, as sensitive header values.
fn headers_of(spec: &AgentSpec) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for (name, value) in spec.headers.reveal()? {
        let n = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| format!("{name:?} is not a header name"))?;
        let mut v = HeaderValue::from_str(value.trim())
            .map_err(|_| format!("the value of header {name:?} is not a header value"))?;
        v.set_sensitive(true);
        map.insert(n, v);
    }
    Ok(map)
}

/// The answer an agent may give to a run.
#[derive(Deserialize, Default)]
struct Done {
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    usage: Usage,
    #[serde(default)]
    cost_usd: Option<f64>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

fn failure(error: String, failure: Option<FailureKind>) -> AgentResult {
    AgentResult {
        failure,
        ..super::fail(error)
    }
}

/// The result of a 2xx answer with `body` (which needn't be JSON: any 2xx means done).
fn done(body: &str) -> AgentResult {
    let d: Done = serde_json::from_str(body).unwrap_or_default();
    let tokens = super::tokens_json(
        d.usage.input_tokens,
        d.usage.output_tokens,
        d.usage.input_tokens + d.usage.output_tokens,
    );
    if d.ok == Some(false) {
        let why = d.error.unwrap_or_else(|| "no reason given".into());
        return AgentResult {
            tokens,
            ..failure(
                format!("the agent says it failed: {why}"),
                Some(FailureKind::Model),
            )
        };
    }
    AgentResult {
        ok: true,
        error: None,
        tokens,
        cost_usd: d.cost_usd,
        cost_source: d.cost_usd.map(|_| COST_SOURCE.into()),
        seconds: 0.0,
        failure: None,
    }
}

/// The result of an answer with `status` and `body`.
fn answered(host: &str, status: u16, body: &str) -> AgentResult {
    if (200..300).contains(&status) {
        return done(body);
    }
    let tail: String = body.trim().chars().take(300).collect();
    failure(
        format!("the agent at {host} answered HTTP {status}: {tail}"),
        None,
    )
}

/// POSTs `body` to the agent; the status and body of its answer.
async fn post(
    http: &reqwest::Client,
    spec: &AgentSpec,
    body: &Value,
) -> Result<(u16, String), String> {
    let url = spec.url.as_deref().unwrap_or_default();
    let host = host_of(url);
    let r = http
        .post(url)
        .headers(headers_of(spec)?)
        .json(body)
        .send()
        .await;
    let r = r.map_err(|e| format!("could not reach the agent at {host}: {}", e.without_url()))?;
    let status = r.status().as_u16();
    let text = r.text().await.map_err(|e| {
        format!(
            "the agent at {host} sent an unreadable answer: {}",
            e.without_url()
        )
    })?;
    Ok((status, text))
}

/// What a hello learnt: the name the agent gives itself, if any.
pub async fn hello(http: &reqwest::Client, spec: &AgentSpec) -> Result<Option<String>, String> {
    let host = host_of(spec.url.as_deref().unwrap_or_default());
    let body = json!({"protocol": PROTOCOL, "type": "hello"});
    let (status, text) = tokio::time::timeout(HELLO_TIMEOUT, post(http, spec, &body))
        .await
        .map_err(|_| {
            format!(
                "the agent at {host} did not answer within {} s",
                HELLO_TIMEOUT.as_secs()
            )
        })??;
    if !(200..300).contains(&status) {
        let tail: String = text.trim().chars().take(300).collect();
        return Err(format!(
            "the agent at {host} answered HTTP {status}: {tail}"
        ));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(v["name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string))
}

impl HttpDriver {
    async fn run_body(&self, run: &AgentRun, token: &str) -> anyhow::Result<Value> {
        let branch = &run.branch;
        let run_id = self.store.branch(&branch.branch_id).await?.run_id;
        let mcp =
            crate::external::endpoints(&self.agent_base, &branch.branch_id, &run.spec.id).mcp_url;
        let deadline = chrono::Utc::now() + self.timeout;
        Ok(json!({
            "protocol": PROTOCOL,
            "type": "run",
            "run_id": run_id,
            "branch_id": branch.branch_id,
            "agent": run.spec.id,
            "task": super::prompt_of(run),
            "mcp_url": mcp,
            "token": token,
            "deadline": deadline.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }))
    }

    async fn run_agent(&self, run: &AgentRun) -> AgentResult {
        if run.resume.is_some() {
            return super::fail("an http agent can't resume in the middle of its run".into());
        }
        let token = run.branch.issue_token(&run.spec.id);
        let result = self.call(run, &token).await;
        run.branch.revoke_token(&run.spec.id);
        result
    }

    async fn call(&self, run: &AgentRun, token: &str) -> AgentResult {
        let body = match self.run_body(run, token).await {
            Ok(b) => b,
            Err(e) => return super::fail(format!("{e:#}")),
        };
        let host = host_of(run.spec.url.as_deref().unwrap_or_default());
        let started = std::time::Instant::now();
        let waited = run
            .branch
            .flow
            .timeout(self.timeout, post(&self.http, &run.spec, &body))
            .await;
        let mut r = match waited {
            None => {
                let secs = self.timeout.as_secs();
                failure(
                    format!("the agent at {host} did not answer within {secs} s"),
                    Some(FailureKind::Model),
                )
            }
            Some(Err(e)) => super::fail(e),
            Some(Ok((status, text))) => answered(&host, status, &text),
        };
        r.seconds = started.elapsed().as_secs_f64();
        r
    }
}

impl Driver for HttpDriver {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move { self.run_agent(&run).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_map_to_results() {
        let ok = answered(
            "a",
            200,
            r#"{"usage": {"input_tokens": 10, "output_tokens": 2}, "cost_usd": 0.01}"#,
        );
        assert!(ok.ok);
        assert_eq!(ok.tokens["total_tokens"], 12);
        assert_eq!(
            (ok.cost_usd, ok.cost_source.as_deref()),
            (Some(0.01), Some(COST_SOURCE))
        );
        assert!(answered("a", 204, "").ok);
        assert!(answered("a", 200, "done").ok);
        let said = answered("a", 200, r#"{"ok": false, "error": "no stock"}"#);
        assert_eq!((said.ok, said.failure), (false, Some(FailureKind::Model)));
        assert_eq!(
            said.error.as_deref(),
            Some("the agent says it failed: no stock")
        );
        let broke = answered("agent.example", 502, "Bad gateway");
        assert_eq!((broke.ok, broke.failure), (false, None));
        assert_eq!(
            broke.error.as_deref(),
            Some("the agent at agent.example answered HTTP 502: Bad gateway")
        );
    }

    #[test]
    fn errors_name_the_host_never_the_whole_url() {
        assert_eq!(
            host_of("https://agent.example:8443/run?key=s3cret"),
            "agent.example:8443"
        );
        assert_eq!(host_of("nonsense"), "the agent");
        assert_eq!(bad_url(Some("https://a.example/run")), None);
        assert!(bad_url(Some("ftp://a.example")).is_some());
        assert!(bad_url(Some("  ")).is_some() && bad_url(None).is_some());
    }
}
