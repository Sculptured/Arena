//! What `/api/providers` and `/api/health` say about each provider: whether it can take calls,
//! why not, and the models it lists.

use futures_util::future::join_all;
use serde::Serialize;

use super::{
    listing::{host_of, list_models, ListFailure},
    Provider, ProviderConfig, ProviderKind,
};

/// Why a provider whose model list was refused for its key is unavailable.
pub const KEY_REJECTED: &str = "the key was rejected";
/// Why `/api/health` doesn't call a local server available: it doesn't ask it.
pub const NOT_CHECKED: &str = "not checked (GET /api/providers asks the server)";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub available: bool,
    /// True when the provider itself was asked (its model list), not only the configuration.
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
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
        ProviderKind::Bedrock => format!(
            "Amazon Bedrock ({})",
            p.id.trim_start_matches(super::BEDROCK_PREFIX)
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
        checked: false,
        why,
        models: None,
    }
}

/// Every provider's status from the configuration alone, without asking any of them: a key
/// tells for the others, but nothing tells whether a local server is up, so it is reported
/// not checked rather than available.
pub fn summary(config: &ProviderConfig) -> Vec<ProviderStatus> {
    let unasked = |p: &Provider| {
        let mut s = configured(p);
        if p.kind == ProviderKind::Local {
            s.available = false;
            s.why = Some(NOT_CHECKED.to_string());
        }
        s
    };
    config.all().iter().map(unasked).collect()
}

/// Every provider's status, with the models of those that list them (each list bounded by
/// [`super::LIST_TIMEOUT`], all asked at once). A key the list is refused for, or a local
/// server that doesn't answer, makes the provider unavailable; a list that fails otherwise
/// leaves it available, with the reason, and the model is typed in.
pub async fn statuses(config: &ProviderConfig, http: &reqwest::Client) -> Vec<ProviderStatus> {
    join_all(config.all().iter().map(|p| status(p, http))).await
}

async fn status(p: &Provider, http: &reqwest::Client) -> ProviderStatus {
    let mut s = configured(p);
    if !s.available || !lists_models(p.kind) {
        return s;
    }
    s.checked = true;
    match list_models(p, http).await {
        Ok(models) => s.models = Some(models),
        Err(ListFailure::Rejected) => {
            s.available = false;
            s.why = Some(KEY_REJECTED.to_string());
        }
        Err(ListFailure::Failed(why)) => {
            s.available = p.kind != ProviderKind::Local;
            s.why = Some(why);
        }
    }
    s
}

fn lists_models(kind: ProviderKind) -> bool {
    matches!(
        kind,
        ProviderKind::OpenAi | ProviderKind::Anthropic | ProviderKind::Local
    )
}
