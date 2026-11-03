//! The OpenAI-compatible chat-completions adapter: OpenRouter, OpenAI and local servers speak
//! the canonical format, so the request goes out as it is (plus OpenRouter's own usage flag)
//! and the response comes back as it is.

use serde_json::{json, Value};

use super::{Provider, ProviderKind, Upstream};

/// The request as this provider takes it: OpenRouter alone is asked to report its cost.
fn outgoing(kind: ProviderKind, request: &Value) -> Value {
    let mut body = request.clone();
    if kind == ProviderKind::OpenRouter {
        body["usage"] = json!({"include": true});
    }
    body
}

/// A request to `path` under the provider's base, with its key as a bearer token when it has one.
pub(super) fn request(
    p: &Provider,
    http: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let url = format!("{}/{path}", p.endpoint.base.trim_end_matches('/'));
    let req = http.request(method, url);
    match &p.endpoint.key {
        Some(key) => req.bearer_auth(key.expose()),
        None => req,
    }
}

pub(super) async fn send(
    p: &Provider,
    http: &reqwest::Client,
    request_body: &Value,
) -> Result<Upstream, String> {
    let r = request(p, http, reqwest::Method::POST, "chat/completions")
        .json(&outgoing(p.kind, request_body))
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {}", e.without_url()))?;
    let status = r.status().as_u16();
    let text = r
        .text()
        .await
        .map_err(|e| format!("upstream response unreadable: {}", e.without_url()))?;
    let response = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    Ok(Upstream {
        status,
        raw: response.clone(),
        response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_openrouter_is_asked_for_usage() {
        let req = json!({"model": "m", "messages": []});
        assert_eq!(
            outgoing(ProviderKind::OpenRouter, &req)["usage"],
            json!({"include": true})
        );
        for kind in [ProviderKind::OpenAi, ProviderKind::Local] {
            assert_eq!(outgoing(kind, &req), req);
        }
    }
}
