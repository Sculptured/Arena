//! Sculptured's HTTP API on one instance's loopback port.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use reqwest::{Client, Method, RequestBuilder, Url};
use serde_json::Value;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What Sculptured answered: its JSON, or an error it reported (`{"error": {code, message}}`).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    Refused { status: u16, message: String },
}

impl Reply {
    /// The JSON of an answer that must not be refused (`what` names the request).
    pub fn expect(self, what: &str) -> Result<Value> {
        match self {
            Reply::Ok(v) => Ok(v),
            Reply::Refused { status, message } => {
                bail!("Sculptured refused {what} (HTTP {status}): {message}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Api {
    client: Client,
    base: Url,
}

impl Api {
    /// The server on 127.0.0.1:`port`.
    pub fn new(port: u16) -> Result<Api> {
        let client = Client::builder().timeout(TIMEOUT).build()?;
        let base = Url::parse(&format!("http://127.0.0.1:{port}/"))?;
        Ok(Api { client, base })
    }

    /// `/api/<segments..>`, each segment percent-encoded (seat ids hold `@`).
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("an http URL has path segments")
            .push("api")
            .extend(segments);
        url
    }

    /// `GET /api/<segments>?<query>`.
    pub async fn get(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Reply> {
        reply(self.client.get(self.url(segments)).query(query)).await
    }

    /// `method /api/<segments>` with `body`, an object, plus a fresh `request_id` (Sculptured
    /// answers a repeated id with the first answer, and a restored store holds old ids).
    pub async fn send(&self, method: Method, segments: &[&str], body: Value) -> Result<Reply> {
        let mut body = body;
        body["request_id"] = Value::String(request_id());
        reply(self.client.request(method, self.url(segments)).json(&body)).await
    }
}

/// An id no request of this program used before.
fn request_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    format!("arena-{}-{nanos}-{n}", std::process::id())
}

/// Sends `r`: a 2xx answer's JSON; an error Sculptured reported as `Refused`; an error when
/// Sculptured can't be reached.
async fn reply(r: RequestBuilder) -> Result<Reply> {
    let resp = r.send().await.context("Sculptured can't be reached")?;
    let status = resp.status().as_u16();
    let text = resp.text().await.context("reading Sculptured's answer")?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(Reply::Ok(body));
    }
    let message = body["error"]["message"]
        .as_str()
        .map_or_else(|| text.trim().to_string(), str::to_string);
    Ok(Reply::Refused { status, message })
}
