//! Asking a provider for its models: `GET <base>/models`, bounded in time and in size, read as
//! the ids of chat models.

use std::time::Duration;

use serde_json::Value;

use super::{anthropic, openai, Provider, ProviderKind};

/// How long a provider's model list may take; the lists are asked for all at once.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(3);
/// The largest model list read; a longer one is not parsed.
pub(super) const MAX_LIST_BYTES: usize = 1024 * 1024;

/// Ids naming models that aren't for chat, in OpenAI's list (which holds every model kind).
const NOT_CHAT: [&str; 12] = [
    "embedding",
    "whisper",
    "tts",
    "transcribe",
    "dall-e",
    "image",
    "moderation",
    "audio",
    "realtime",
    "sora",
    "davinci",
    "babbage",
];

/// Why a model list didn't come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ListFailure {
    /// 401 or 403: the provider turned the key down.
    Rejected,
    /// Anything else, as a plain reason.
    Failed(String),
}

/// `host[:port]` of a base URL: what a label or a reason may show of it (never credentials).
pub(super) fn host_of(base: &str) -> String {
    reqwest::Url::parse(base)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_string();
            Some(match u.port() {
                Some(p) => format!("{host}:{p}"),
                None => host,
            })
        })
        .unwrap_or_else(|| "the server".to_string())
}

/// The provider's model-list request, with its own way of sending the key.
fn list_request(p: &Provider, http: &reqwest::Client) -> reqwest::RequestBuilder {
    match p.kind {
        ProviderKind::Anthropic => anthropic::request(p, http, reqwest::Method::GET, "models")
            // The Models API pages at 20 by default; 1000 is its largest page.
            .query(&[("limit", "1000")]),
        _ => openai::request(p, http, reqwest::Method::GET, "models"),
    }
}

/// The chat models the provider lists, sorted; within [`LIST_TIMEOUT`].
pub(super) async fn list_models(
    p: &Provider,
    http: &reqwest::Client,
) -> Result<Vec<String>, ListFailure> {
    let host = host_of(&p.endpoint.base);
    let listed = tokio::time::timeout(LIST_TIMEOUT, ask(p, http, &host))
        .await
        .map_err(|_| {
            let secs = LIST_TIMEOUT.as_secs();
            ListFailure::Failed(format!("{host} did not answer within {secs} s"))
        })??;
    Ok(chat_models(p.kind, model_ids(&listed)))
}

/// The model list as JSON, read up to [`MAX_LIST_BYTES`].
async fn ask(p: &Provider, http: &reqwest::Client, host: &str) -> Result<Value, ListFailure> {
    let failed = |why: String| ListFailure::Failed(why);
    let r = list_request(p, http)
        .send()
        .await
        .map_err(|e| failed(format!("{host} did not answer: {}", e.without_url())))?;
    match r.status().as_u16() {
        401 | 403 => return Err(ListFailure::Rejected),
        s if !(200..300).contains(&s) => {
            return Err(failed(format!("{host} answered {s} to the model list")))
        }
        _ => {}
    }
    let body = read_capped(r, MAX_LIST_BYTES)
        .await
        .map_err(|why| failed(format!("{host} {why}")))?;
    serde_json::from_slice(&body)
        .map_err(|_| failed(format!("{host} sent a model list that is not JSON")))
}

/// The body of `r`, refused once it passes `cap` bytes (never read further).
async fn read_capped(mut r: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = r
        .chunk()
        .await
        .map_err(|e| format!("sent an unreadable model list: {}", e.without_url()))?
    {
        if body.len() + chunk.len() > cap {
            return Err(format!("sent a model list over {} KB", cap / 1024));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The `data[].id` of a model list (OpenAI's and the Claude API's alike), sorted.
fn model_ids(list: &Value) -> Vec<String> {
    let mut ids: Vec<String> = list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    ids.sort();
    ids
}

/// The ids of chat models among `ids`: OpenAI's list loses its embedding, audio, image and
/// other non-chat models; other providers' lists are kept whole.
pub(super) fn chat_models(kind: ProviderKind, ids: Vec<String>) -> Vec<String> {
    if kind != ProviderKind::OpenAi {
        return ids;
    }
    ids.into_iter()
        .filter(|id| !NOT_CHAT.iter().any(|w| id.contains(w)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model list over the bound is refused without reading the rest.
    #[tokio::test]
    async fn a_list_over_the_bound_is_refused() {
        let answer = |n: usize| reqwest::Response::from(axum::http::Response::new(vec![b'x'; n]));
        assert_eq!(read_capped(answer(10), 10).await.unwrap().len(), 10);
        let refused = read_capped(answer(MAX_LIST_BYTES + 1), MAX_LIST_BYTES).await;
        assert_eq!(refused.unwrap_err(), "sent a model list over 1024 KB");
    }

    #[test]
    fn hosts_never_show_credentials() {
        assert_eq!(
            host_of("http://user:pw@127.0.0.1:11434/v1"),
            "127.0.0.1:11434"
        );
        assert_eq!(host_of("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_of("nonsense"), "the server");
    }

    #[test]
    fn model_ids_are_read_and_sorted() {
        let list = serde_json::json!({"data": [{"id": "b"}, {"id": "a"}, {"x": 1}]});
        assert_eq!(model_ids(&list), ["a", "b"]);
        assert!(model_ids(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn only_openais_list_loses_its_non_chat_models() {
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = ids(&[
            "gpt-5",
            "o4-mini",
            "text-embedding-3-large",
            "whisper-1",
            "tts-1-hd",
            "gpt-4o-transcribe",
            "dall-e-3",
            "gpt-image-1",
            "omni-moderation-latest",
            "gpt-4o-audio-preview",
            "gpt-realtime",
            "sora-2",
            "davinci-002",
            "babbage-002",
        ]);
        assert_eq!(
            chat_models(ProviderKind::OpenAi, all.clone()),
            ["gpt-5", "o4-mini"]
        );
        for kind in [
            ProviderKind::Local,
            ProviderKind::Anthropic,
            ProviderKind::OpenRouter,
        ] {
            assert_eq!(chat_models(kind, all.clone()), all);
        }
    }
}
