//! What `/api/providers` and `/api/health` say about each provider: whether it can take calls,
//! why not, and the models it lists.

use std::time::Duration;

use futures_util::future::join_all;
use serde::Serialize;
use serde_json::Value;

use super::{openai, Provider, ProviderConfig, ProviderKind};

/// How long a provider's model list may take; the lists are asked for all at once.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

/// `host[:port]` of a base URL: what a label or a reason may show of it (never credentials).
fn host_of(base: &str) -> String {
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

fn label(p: &Provider) -> String {
    match p.kind {
        ProviderKind::OpenRouter => "OpenRouter".into(),
        ProviderKind::OpenAi => "OpenAI".into(),
        ProviderKind::Anthropic => "Anthropic (Claude)".into(),
        ProviderKind::Local => format!(
            "{} (local, {})",
            p.id.trim_start_matches(super::LOCAL_PREFIX),
            host_of(&p.endpoint.base)
        ),
    }
}

/// The status from the configuration alone: keys present, local servers configured.
fn configured(p: &Provider) -> ProviderStatus {
    let why = p.unavailable().map(|u| u.why());
    ProviderStatus {
        id: p.id.clone(),
        kind: p.kind,
        label: label(p),
        available: why.is_none(),
        why,
        models: None,
    }
}

/// Every provider's status from the configuration alone, without asking any of them.
pub fn summary(config: &ProviderConfig) -> Vec<ProviderStatus> {
    config.all().iter().map(configured).collect()
}

/// Every provider's status, with the models of those that list them (each list bounded by
/// [`LIST_TIMEOUT`], all asked at once). A local server that doesn't answer is unavailable;
/// a list that fails elsewhere leaves the provider available, with the reason, and the model
/// is typed in.
pub async fn statuses(config: &ProviderConfig, http: &reqwest::Client) -> Vec<ProviderStatus> {
    join_all(config.all().iter().map(|p| status(p, http))).await
}

async fn status(p: &Provider, http: &reqwest::Client) -> ProviderStatus {
    let mut s = configured(p);
    if !s.available || !lists_models(p.kind) {
        return s;
    }
    match list_models(p, http).await {
        Ok(models) => s.models = Some(models),
        Err(why) => {
            s.available = p.kind != ProviderKind::Local;
            s.why = Some(why);
        }
    }
    s
}

fn lists_models(kind: ProviderKind) -> bool {
    matches!(kind, ProviderKind::OpenAi | ProviderKind::Local)
}

/// The ids in the provider's `GET <base>/models`, sorted.
async fn list_models(p: &Provider, http: &reqwest::Client) -> Result<Vec<String>, String> {
    let host = host_of(&p.endpoint.base);
    let ask = async {
        let r = openai::request(p, http, reqwest::Method::GET, "models")
            .send()
            .await
            .map_err(|e| format!("{host} did not answer: {}", e.without_url()))?;
        if !r.status().is_success() {
            return Err(format!("{host} answered {} to the model list", r.status()));
        }
        r.json::<Value>()
            .await
            .map_err(|_| format!("{host} sent a model list that is not JSON"))
    };
    let listed = tokio::time::timeout(LIST_TIMEOUT, ask)
        .await
        .map_err(|_| format!("{host} did not answer within {} s", LIST_TIMEOUT.as_secs()))??;
    Ok(model_ids(&listed))
}

/// The `data[].id` of an OpenAI-style model list, sorted.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
