//! One Twenty server's REST API and health, with the workspace's API key.
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::Value;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(60);

/// What the server answered: its `data`, or why it refused (a tool error, not a failure of the
/// integration).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    Refused(String),
}

#[derive(Debug, Clone)]
pub struct Rest {
    client: Client,
    base: String,
    key: String,
}

/// The message in a refusal: Twenty's `messages`, else the status.
fn refusal(status: u16, body: &Value) -> String {
    let messages = body["messages"].as_array().into_iter().flatten();
    let text: Vec<&str> = messages.filter_map(Value::as_str).collect();
    if text.is_empty() {
        format!("Twenty answered HTTP {status}")
    } else {
        text.join("; ")
    }
}

fn is_gateway_error(status: u16) -> bool {
    matches!(status, 502..=504)
}

impl Rest {
    pub fn new(port: u16, key: &str) -> Result<Rest> {
        Ok(Rest {
            client: Client::builder().timeout(TIMEOUT).build()?,
            base: format!("http://127.0.0.1:{port}"),
            key: key.into(),
        })
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}/rest/{path}", self.base))
            .bearer_auth(&self.key)
    }

    pub async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Reply> {
        reply(self.request(Method::GET, path).query(query)).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Reply> {
        reply(self.request(Method::POST, path).json(body)).await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Result<Reply> {
        reply(self.request(Method::PATCH, path).json(body)).await
    }
}

/// Sends `r`: a 2xx answer's `data`; a refusal as `Refused`; an error when Twenty can't be reached.
async fn reply(r: RequestBuilder) -> Result<Reply> {
    let resp = r.send().await.context("Twenty can't be reached")?;
    let status = resp.status().as_u16();
    if is_gateway_error(status) {
        anyhow::bail!("Twenty is not answering (HTTP {status})");
    }
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Ok(Reply::Refused(refusal(status, &body)));
    }
    Ok(Reply::Ok(body["data"].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refusal_says_what_twenty_said() {
        let body = json!({"messages": ["no such field", "and another"]});
        assert_eq!(refusal(400, &body), "no such field; and another");
        assert_eq!(refusal(401, &json!({})), "Twenty answered HTTP 401");
    }
}
