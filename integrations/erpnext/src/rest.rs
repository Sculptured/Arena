//! Frappe's REST API on the stack's frontend, one site per request (its `Host` header).
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::header::{AUTHORIZATION, HOST};
use reqwest::{Client, Method, RequestBuilder, Url};
use serde_json::Value;

use crate::frappe::{error_text, ApiKey};

/// How long one request may take (a big submit can be slow).
const TIMEOUT: Duration = Duration::from_secs(300);

/// What a site answered: its result, or an error Frappe reported (a tool error, not a failure
/// of the integration).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    /// Frappe's HTTP status and message.
    Refused {
        status: u16,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct Rest {
    client: Client,
    base: Url,
    auth: Option<String>,
}

impl Rest {
    /// The frontend on 127.0.0.1:`port`, without a key.
    pub fn new(port: u16) -> Result<Rest> {
        let client = Client::builder().timeout(TIMEOUT).build()?;
        let base = Url::parse(&format!("http://127.0.0.1:{port}/"))?;
        Ok(Rest {
            client,
            base,
            auth: None,
        })
    }

    /// The same, sending `key` with every request.
    pub fn with_key(&self, key: &ApiKey) -> Rest {
        Rest {
            auth: Some(key.header()),
            ..self.clone()
        }
    }

    /// `/api/<segments..>`, each segment percent-encoded (names may hold spaces and `/`).
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("an http URL has path segments")
            .push("api")
            .extend(segments);
        url
    }

    fn request(&self, method: Method, site: &str, segments: &[&str]) -> RequestBuilder {
        let r = self
            .client
            .request(method, self.url(segments))
            .header(HOST, site);
        match &self.auth {
            Some(a) => r.header(AUTHORIZATION, a),
            None => r,
        }
    }

    /// `GET /api/<segments>?<query>` on `site`.
    pub async fn get(
        &self,
        site: &str,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<Reply> {
        reply(self.request(Method::GET, site, segments).query(query)).await
    }

    /// `method /api/<segments>` on `site` with a JSON body.
    pub async fn send(
        &self,
        method: Method,
        site: &str,
        segments: &[&str],
        body: &Value,
    ) -> Result<Reply> {
        reply(self.request(method, site, segments).json(body)).await
    }

    /// Whether the frontend and the backend behind it answer (any site, any answer but a
    /// gateway error).
    pub async fn answers(&self) -> bool {
        let r = self
            .request(Method::GET, "localhost", &["method", "ping"])
            .send()
            .await;
        r.is_ok_and(|r| !is_gateway_error(r.status().as_u16()))
    }
}

fn is_gateway_error(status: u16) -> bool {
    matches!(status, 502..=504)
}

/// Sends `r`: a 2xx answer's `data` (resources) or `message` (methods); a Frappe error as
/// `Refused`; an error when ERPNext can't be reached.
async fn reply(r: RequestBuilder) -> Result<Reply> {
    let resp = r.send().await.context("ERPNext can't be reached")?;
    let status = resp.status().as_u16();
    if is_gateway_error(status) {
        bail!("ERPNext is not answering (HTTP {status})");
    }
    let text = resp.text().await.context("reading ERPNext's answer")?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = error_text(status, &body);
        return Ok(Reply::Refused { status, message });
    }
    let data = ["data", "message"]
        .iter()
        .find_map(|k| body.get(*k).cloned());
    Ok(Reply::Ok(data.unwrap_or(body)))
}
