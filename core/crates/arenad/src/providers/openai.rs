//! The OpenAI-compatible chat-completions adapter: OpenRouter, OpenAI and local servers speak
//! the canonical format, so the request goes out as it is (plus OpenRouter's own usage flag)
//! and the response comes back as it is.

use serde_json::{json, Value};

use super::{anthropic, transport, Provider, ProviderKind, Upstream};

/// The request as this provider takes it: canonical fields only (an agent moved here by a fork
/// may carry Claude's own content), and OpenRouter alone is asked to report its cost.
fn outgoing(kind: ProviderKind, request: &Value) -> Value {
    let mut body = request.clone();
    anthropic::strip_extras(&mut body);
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
    let req = http.request(method, transport::url(&p.endpoint.base, path));
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
    let req = request(p, http, reqwest::Method::POST, "chat/completions")
        .json(&outgoing(p.kind, request_body));
    let answer = transport::exchange(req).await?;
    Ok(Upstream {
        status: answer.status,
        raw: answer.body.clone(),
        response: answer.body,
        retry_after: answer.retry_after,
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

    #[test]
    fn claude_content_is_not_sent_to_other_providers() {
        let msg = json!({"role": "assistant", "content": "hi"});
        let mut moved = msg.clone();
        moved["anthropic_content"] =
            json!([{"type": "thinking", "thinking": "", "signature": "s"}]);
        moved["anthropic_context"] = json!("abc");
        let req = json!({"model": "m", "messages": [moved]});
        let sent = outgoing(ProviderKind::OpenAi, &req);
        assert_eq!(sent["messages"], json!([msg]));
    }
}
