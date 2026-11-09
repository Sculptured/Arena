//! The Amazon Bedrock adapter (provider `bedrock:<region>`): a canonical chat-completions request
//! goes out as a Converse API call (`POST <base>/model/<model>/converse`), signed with the user's
//! own AWS credentials ([`credentials`]), and the answer comes back canonical ([`converse`]).
//!
//! Untested against AWS itself: its tests run against a local server standing in for Bedrock.

pub(crate) mod converse;
pub(crate) mod credentials;
mod sigv4;

use chrono::Utc;
use serde_json::Value;

use super::{transport, Provider, Upstream, BEDROCK_PREFIX};
use credentials::Auth;

/// Where Bedrock Runtime answers in `region`.
pub(super) fn base_url(region: &str) -> String {
    format!("https://bedrock-runtime.{region}.amazonaws.com")
}

/// True for a region name AWS could have: lower-case letters, digits and dashes.
pub(super) fn valid_region(region: &str) -> bool {
    !region.is_empty()
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn region_of(p: &Provider) -> &str {
    p.id.strip_prefix(BEDROCK_PREFIX).unwrap_or_default()
}

/// Sends one canonical request to Bedrock, signed with the credentials found now.
pub(super) async fn send(
    p: &Provider,
    http: &reqwest::Client,
    chat: &Value,
) -> Result<Upstream, String> {
    let auth = credentials::resolve().await?;
    send_with(p, http, chat, &auth).await
}

const JSON: &str = "application/json";

/// `host[:port]` of `url`, as its `Host` header carries it.
fn host_of(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("bad Bedrock address: {e}"))?;
    match (parsed.host_str(), parsed.port()) {
        (Some(h), Some(port)) => Ok(format!("{h}:{port}")),
        (Some(h), None) => Ok(h.to_string()),
        (None, _) => Err("bad Bedrock address: no host".into()),
    }
}

/// The headers that prove who sends `req`: a bearer token, or a SigV4 signature for `region`.
fn auth_headers(
    region: &str,
    auth: &Auth,
    req: &sigv4::Request<'_>,
) -> Vec<(&'static str, String)> {
    match auth {
        Auth::Bearer(token) => vec![("authorization", format!("Bearer {}", token.expose()))],
        Auth::Keys(keys) => {
            let scope = sigv4::Scope {
                region,
                service: "bedrock",
            };
            sigv4::sign(req, keys, &scope, Utc::now())
        }
    }
}

/// The request for `chat`, signed by `auth`.
fn signed(
    p: &Provider,
    http: &reqwest::Client,
    chat: &Value,
    auth: &Auth,
) -> Result<reqwest::RequestBuilder, String> {
    let model = chat["model"].as_str().unwrap_or_default();
    let path = format!("/model/{}/converse", sigv4::uri_encode(model));
    let url = transport::url(&p.endpoint.base, path.trim_start_matches('/'));
    let payload = serde_json::to_vec(&converse::request(chat)).map_err(|e| e.to_string())?;
    let host = host_of(&url)?;
    let mut req = http.post(url).header("content-type", JSON);
    let r = sigv4::Request {
        method: "POST",
        host: &host,
        path: &path,
        payload: &payload,
        content_type: Some(JSON),
    };
    for (k, v) in auth_headers(region_of(p), auth, &r) {
        let mut v = reqwest::header::HeaderValue::from_str(&v).map_err(|e| e.to_string())?;
        v.set_sensitive(k != "x-amz-date");
        req = req.header(k, v);
    }
    Ok(req.body(payload))
}

pub(super) async fn send_with(
    p: &Provider,
    http: &reqwest::Client,
    chat: &Value,
    auth: &Auth,
) -> Result<Upstream, String> {
    let answer = transport::exchange(signed(p, http, chat, auth)?).await?;
    let raw = answer.body;
    let ok = (200..300).contains(&answer.status);
    let model = chat["model"].as_str().unwrap_or_default();
    let response = match ok {
        true => converse::response(&raw, model),
        false => converse::error(&raw),
    };
    Ok(Upstream {
        status: answer.status,
        response,
        raw,
        retry_after: answer.retry_after,
    })
}

#[cfg(test)]
mod tests {
    use super::credentials::Keys;
    use super::*;
    use crate::providers::{Endpoint, ProviderKind, Secret};
    use axum::{http::HeaderMap, routing::post, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(String, HeaderMap, Value)>>>;

    /// A local server standing in for Bedrock Runtime: it keeps each request and answers with
    /// `status` and `body`.
    async fn fake_bedrock(status: u16, body: Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let keep = seen.clone();
        let app = Router::new().route(
            "/{*path}",
            post(
                move |uri: axum::http::Uri, headers: HeaderMap, Json(req): Json<Value>| {
                    let keep = keep.clone();
                    let body = body.clone();
                    async move {
                        keep.lock()
                            .unwrap()
                            .push((uri.path().to_string(), headers, req));
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            Json(body),
                        )
                    }
                },
            ),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, seen)
    }

    fn provider(base: &str) -> Provider {
        Provider {
            id: "bedrock:us-east-1".into(),
            kind: ProviderKind::Bedrock,
            endpoint: Endpoint::new(base, None),
            max_tokens: 1024,
        }
    }

    fn keys() -> Auth {
        Auth::Keys(Keys {
            access_key: "AKIDTEST".into(),
            secret: Secret::new("secret-key"),
            session_token: Some(Secret::new("session")),
            expires: None,
        })
    }

    #[tokio::test]
    async fn a_call_is_signed_sent_as_converse_and_answered_canonically() {
        let answer = json!({"output": {"message": {"role": "assistant", "content": [{"text": "hi"}]}},
                            "stopReason": "end_turn", "usage": {"inputTokens": 3, "outputTokens": 1}});
        let (base, seen) = fake_bedrock(200, answer).await;
        let chat = json!({"model": "us.anthropic.claude-x:0", "messages": [{"role": "user", "content": "ping"}]});
        let u = send_with(&provider(&base), &reqwest::Client::new(), &chat, &keys())
            .await
            .unwrap();
        assert_eq!(u.status, 200);
        assert_eq!(u.response["choices"][0]["message"]["content"], "hi");
        assert_eq!(u.response["usage"]["total_tokens"], 4);
        let seen = seen.lock().unwrap();
        let (path, headers, body) = &seen[0];
        assert_eq!(path, "/model/us.anthropic.claude-x%3A0/converse");
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": [{"text": "ping"}]}])
        );
        let auth = headers["authorization"].to_str().unwrap();
        assert!(
            auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDTEST/"),
            "{auth}"
        );
        assert!(auth.contains("/us-east-1/bedrock/aws4_request"), "{auth}");
        assert!(!auth.contains("secret-key"));
        assert_eq!(headers["x-amz-security-token"], "session");
    }

    #[tokio::test]
    async fn an_api_key_goes_as_a_bearer_token_and_errors_keep_their_message() {
        let (base, seen) = fake_bedrock(
            400,
            json!({"message": "The provided model identifier is invalid."}),
        )
        .await;
        let chat = json!({"model": "nope", "messages": [{"role": "user", "content": "ping"}]});
        let auth = Auth::Bearer(Secret::new("bedrock-key"));
        let u = send_with(&provider(&base), &reqwest::Client::new(), &chat, &auth)
            .await
            .unwrap();
        assert_eq!(u.status, 400);
        assert_eq!(
            u.response["error"]["message"],
            "The provided model identifier is invalid."
        );
        assert_eq!(
            seen.lock().unwrap()[0].1["authorization"],
            "Bearer bedrock-key"
        );
    }

    #[test]
    fn regions_are_checked_before_they_make_an_address() {
        assert!(valid_region("eu-central-1"));
        assert!(!valid_region("") && !valid_region("evil.com/x") && !valid_region("US-EAST-1"));
        assert_eq!(
            base_url("us-west-2"),
            "https://bedrock-runtime.us-west-2.amazonaws.com"
        );
    }
}
