//! The Claude API adapter: a canonical chat-completions request goes out as a native Messages
//! API request (`POST <base>/messages`), and the answer comes back canonical. The translation
//! both ways is pure ([`request`], [`response`]); this module only sends.
//!
//! Claude's own content blocks (its thinking among them) can't be told in the canonical
//! message, yet must go back unchanged in later requests of the conversation. They travel on
//! the canonical assistant message, as [`CONTENT_KEY`], next to [`CONTEXT_KEY`]: a fingerprint
//! of the system prompt and tools they were made under.

mod request;
mod response;

use reqwest::header::HeaderValue;
use serde_json::Value;

use super::{openai, Provider, Upstream};

/// The `anthropic-version` every request sends.
pub(super) const VERSION: &str = "2026-10-21";
/// The canonical assistant message's copy of Claude's content blocks, as they came.
pub(super) const CONTENT_KEY: &str = "anthropic_content";
/// The fingerprint of the system prompt and tools [`CONTENT_KEY`]'s blocks were made under.
pub(super) const CONTEXT_KEY: &str = "anthropic_context";

/// A request to `path` under the provider's base, with the API version and, when it has one,
/// the key (as a sensitive header, like a bearer token).
pub(super) fn request(
    p: &Provider,
    http: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let url = format!("{}/{path}", p.endpoint.base.trim_end_matches('/'));
    let req = http
        .request(method, url)
        .header("anthropic-version", VERSION);
    let key = p.endpoint.key.as_ref().and_then(|k| {
        let mut v = HeaderValue::from_str(k.expose()).ok()?;
        v.set_sensitive(true);
        Some(v)
    });
    match key {
        Some(v) => req.header("x-api-key", v),
        None => req,
    }
}

/// Sends one canonical request as a Messages API call. A 2xx message is translated to the
/// canonical response; any other answer (an API error `{type: "error", error: {type,
/// message}}`) is kept as it came, with its status, for the gateway to report.
pub(super) async fn send(
    p: &Provider,
    http: &reqwest::Client,
    chat: &Value,
) -> Result<Upstream, String> {
    let body = request::translate(chat);
    let context = request::context_of(&body);
    let req = request(p, http, reqwest::Method::POST, "messages").json(&body);
    let (status, raw) = openai::exchange(req).await?;
    let is_message = (200..300).contains(&status) && raw["type"] == "message";
    let response = if is_message {
        response::translate(&raw, &context)
    } else {
        raw.clone()
    };
    Ok(Upstream {
        status,
        response,
        raw,
    })
}

/// Removes Claude's own content from the canonical messages of `request`, for a provider that
/// takes canonical fields only (an agent a fork moved off Claude carries them).
pub(super) fn strip_extras(request: &mut Value) {
    let messages = request.get_mut("messages").and_then(Value::as_array_mut);
    for m in messages
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        m.remove(CONTENT_KEY);
        m.remove(CONTEXT_KEY);
    }
}
