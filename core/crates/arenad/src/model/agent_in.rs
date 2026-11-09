//! A team member as it is read: [`super::AgentSpec`]'s fields, plus the Bedrock form.

use serde::Deserialize;

use super::{AgentKind, AgentSpec, Headers};
use crate::providers::BEDROCK_PREFIX;

/// The kinds a team member may name: every [`AgentKind`], and `bedrock`, which is an `open`
/// agent whose model calls go to Amazon Bedrock.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindIn {
    ClaudeCode,
    Codex,
    Open,
    External,
    Http,
    Bedrock,
}

#[derive(Deserialize)]
pub(super) struct AgentSpecIn {
    id: String,
    name: String,
    kind: KindIn,
    #[serde(default)]
    model: Option<String>,
    /// A Bedrock agent's model (`model` means the same).
    #[serde(default)]
    model_id: Option<String>,
    /// A Bedrock agent's AWS region.
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    lead: bool,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: Headers,
}

impl From<AgentSpecIn> for AgentSpec {
    fn from(a: AgentSpecIn) -> AgentSpec {
        let (kind, provider) = match a.kind {
            KindIn::ClaudeCode => (AgentKind::ClaudeCode, a.provider),
            KindIn::Codex => (AgentKind::Codex, a.provider),
            KindIn::Open => (AgentKind::Open, a.provider),
            KindIn::External => (AgentKind::External, a.provider),
            KindIn::Http => (AgentKind::Http, a.provider),
            KindIn::Bedrock => {
                let region = a.region.as_deref().map(str::trim).unwrap_or_default();
                (AgentKind::Open, Some(format!("{BEDROCK_PREFIX}{region}")))
            }
        };
        AgentSpec {
            id: a.id,
            name: a.name,
            kind,
            model: a.model.or(a.model_id),
            prompt: a.prompt,
            provider,
            read_only: a.read_only,
            lead: a.lead,
            url: a.url,
            headers: a.headers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bedrock_agent_is_an_open_agent_on_bedrock() {
        let a: AgentSpec =
            serde_json::from_value(json!({"id": "a", "name": "A", "kind": "bedrock",
            "model_id": "anthropic.claude-x", "region": " eu-west-1 "}))
            .unwrap();
        assert_eq!(a.kind, AgentKind::Open);
        assert_eq!(a.provider.as_deref(), Some("bedrock:eu-west-1"));
        assert_eq!(a.model.as_deref(), Some("anthropic.claude-x"));
        let back = serde_json::to_value(&a).unwrap();
        assert_eq!(
            (back["kind"].clone(), back.get("model_id")),
            (json!("open"), None)
        );
    }

    #[test]
    fn an_http_agent_keeps_its_url_and_writes_no_header_value() {
        let a: AgentSpec = serde_json::from_value(json!({"id": "a", "name": "A", "kind": "http",
            "url": "https://agent.example/run", "headers": {"Authorization": "Bearer tok"}}))
        .unwrap();
        assert_eq!(a.kind, AgentKind::Http);
        let back = serde_json::to_string(&a).unwrap();
        assert!(
            back.contains("https://agent.example/run") && !back.contains("tok\""),
            "{back}"
        );
        assert_eq!(serde_json::from_str::<AgentSpec>(&back).unwrap(), a);
    }
}
