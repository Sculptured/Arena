//! Model providers: where an open agent's model calls go, and how the gateway talks to each.
//!
//! The recorded, canonical format is OpenAI chat completions. Each provider takes a canonical
//! request and gives back a canonical response plus its own raw response ([`Provider::send`]);
//! the agent loop, replay and fork only ever see the canonical form.

mod anthropic;
mod config;
mod listing;
mod openai;
mod status;

use serde::Serialize;
use serde_json::Value;

pub use config::{
    is_openrouter_url, local_key_env, Endpoint, LocalEndpoint, ProviderConfig, Secret,
    ANTHROPIC_BASE, LOCAL_KEY_PREFIX, LOCAL_MODELS_ENV, OPENAI_BASE, OPENROUTER_BASE,
};
pub use listing::LIST_TIMEOUT;
pub use status::{statuses, summary, ProviderStatus, KEY_REJECTED, NOT_CHECKED};

pub const OPENROUTER: &str = "openrouter";
pub const OPENAI: &str = "openai";
pub const ANTHROPIC: &str = "anthropic";
pub const LOCAL_PREFIX: &str = "local:";
/// The local provider the old single-upstream override becomes.
pub const CUSTOM_LOCAL: &str = "custom";
/// The model of an OpenRouter agent that names none.
pub const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-chat-v3.1";
pub const LOCAL_COST_SOURCE: &str = "local (no API charge)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenRouter,
    OpenAi,
    Anthropic,
    Local,
}

/// Why a provider can't take calls, judged from the configuration alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The env var holding its key is not set.
    MissingKey(&'static str),
}

impl Unavailable {
    /// The plain reason, as `/api/providers` shows it.
    pub fn why(&self) -> String {
        match self {
            Unavailable::MissingKey(env) => format!("{env} is not set"),
        }
    }

    /// Why a run or fork that needs the provider is refused.
    pub fn refusal(&self) -> String {
        match self {
            Unavailable::MissingKey(env) => format!("open mode needs {env}"),
        }
    }
}

/// What a provider answered: the HTTP status, the response in the canonical (OpenAI
/// chat-completions) shape, and the provider's own response as it came.
#[derive(Debug, Clone, PartialEq)]
pub struct Upstream {
    pub status: u16,
    pub response: Value,
    pub raw: Value,
}

/// One provider, resolved from the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    /// `openrouter`, `openai`, `anthropic` or `local:<name>`.
    pub id: String,
    pub kind: ProviderKind,
    pub endpoint: Endpoint,
}

impl Provider {
    fn new(id: impl Into<String>, kind: ProviderKind, endpoint: &Endpoint) -> Provider {
        Provider {
            id: id.into(),
            kind,
            endpoint: endpoint.clone(),
        }
    }

    /// Sends one canonical chat-completions `request`. `Err` is a transport failure (no answer).
    pub async fn send(&self, http: &reqwest::Client, request: &Value) -> Result<Upstream, String> {
        match self.kind {
            ProviderKind::Anthropic => anthropic::send(self, http, request).await,
            _ => openai::send(self, http, request).await,
        }
    }

    /// Why the provider can't take calls (no request is made to find out).
    pub fn unavailable(&self) -> Option<Unavailable> {
        let missing = |env| {
            self.endpoint
                .key
                .is_none()
                .then_some(Unavailable::MissingKey(env))
        };
        match self.kind {
            ProviderKind::OpenRouter => missing("OPENROUTER_API_KEY"),
            ProviderKind::OpenAi => missing("OPENAI_API_KEY"),
            ProviderKind::Anthropic => missing("ANTHROPIC_API_KEY"),
            ProviderKind::Local => None,
        }
    }

    /// The cost of a successful call with `usage`: what OpenRouter reports, 0 on a local
    /// server, unknown otherwise (no price table is kept).
    pub fn cost(&self, usage: &Value) -> Option<f64> {
        match self.kind {
            ProviderKind::OpenRouter => usage.get("cost").and_then(Value::as_f64),
            ProviderKind::Local => Some(0.0),
            ProviderKind::OpenAi | ProviderKind::Anthropic => None,
        }
    }

    /// The label of a cost [`Provider::cost`] knows; `None` where it never knows one.
    pub fn cost_source(&self) -> Option<&'static str> {
        match self.kind {
            ProviderKind::OpenRouter => Some(OPENROUTER),
            ProviderKind::Local => Some(LOCAL_COST_SOURCE),
            ProviderKind::OpenAi | ProviderKind::Anthropic => None,
        }
    }

    /// The model an agent that names none gets: only OpenRouter has one.
    pub fn default_model(&self) -> Option<&'static str> {
        (self.kind == ProviderKind::OpenRouter).then_some(DEFAULT_OPENROUTER_MODEL)
    }
}

impl ProviderConfig {
    /// The provider `id` names (`None`: the default one); `Err` names the known ones.
    pub fn resolve(&self, id: Option<&str>) -> Result<Provider, String> {
        let id = self.id_of(id);
        let found = match id {
            OPENROUTER => Some(Provider::new(
                id,
                ProviderKind::OpenRouter,
                &self.openrouter,
            )),
            OPENAI => Some(Provider::new(id, ProviderKind::OpenAi, &self.openai)),
            ANTHROPIC => Some(Provider::new(id, ProviderKind::Anthropic, &self.anthropic)),
            _ => id
                .strip_prefix(LOCAL_PREFIX)
                .and_then(|name| self.local(name))
                .map(|l| Provider::new(id, ProviderKind::Local, &l.endpoint)),
        };
        found.ok_or_else(|| {
            let known: Vec<String> = self.all().into_iter().map(|p| p.id).collect();
            format!("unknown provider {id:?} (known: {})", known.join(", "))
        })
    }

    /// The id `provider` stands for: itself, or the default provider when absent (what
    /// [`ProviderConfig::resolve`] goes by).
    pub fn id_of<'a>(&'a self, provider: Option<&'a str>) -> &'a str {
        provider.unwrap_or(&self.default)
    }

    /// Every configured provider: OpenRouter, OpenAI, Anthropic, then each local server.
    pub fn all(&self) -> Vec<Provider> {
        let mut out = vec![
            Provider::new(OPENROUTER, ProviderKind::OpenRouter, &self.openrouter),
            Provider::new(OPENAI, ProviderKind::OpenAi, &self.openai),
            Provider::new(ANTHROPIC, ProviderKind::Anthropic, &self.anthropic),
        ];
        out.extend(self.locals.iter().map(|l| {
            Provider::new(
                format!("{LOCAL_PREFIX}{}", l.name),
                ProviderKind::Local,
                &l.endpoint,
            )
        }));
        out
    }

    /// True when some provider can take calls, judged from the configuration alone.
    pub fn any_available(&self) -> bool {
        self.all().iter().any(|p| p.unavailable().is_none())
    }

    /// Why an open agent on `provider` with `model` can't run, if it can't.
    pub fn refusal(&self, provider: Option<&str>, model: Option<&str>) -> Option<String> {
        let p = match self.resolve(provider) {
            Ok(p) => p,
            Err(e) => return Some(e),
        };
        if let Some(u) = p.unavailable() {
            return Some(u.refusal());
        }
        let has_model = model.is_some_and(|m| !m.trim().is_empty());
        (!has_model && p.default_model().is_none())
            .then(|| format!("provider {} has no default model: name one", p.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProviderConfig {
        let mut c = ProviderConfig::default();
        c.openrouter.key = Some(Secret::new("k"));
        c.locals.push(LocalEndpoint {
            name: "box".into(),
            endpoint: Endpoint::new("http://127.0.0.1:9/v1", None),
        });
        c
    }

    #[test]
    fn ids_resolve_to_their_provider() {
        let c = config();
        assert_eq!(c.resolve(None).unwrap().kind, ProviderKind::OpenRouter);
        assert_eq!(
            c.resolve(Some("openai")).unwrap().kind,
            ProviderKind::OpenAi
        );
        let local = c.resolve(Some("local:box")).unwrap();
        assert_eq!(
            (local.kind, local.endpoint.base.as_str()),
            (ProviderKind::Local, "http://127.0.0.1:9/v1")
        );
        let e = c.resolve(Some("local:nope")).unwrap_err();
        assert!(e.contains("local:nope") && e.contains("local:box"), "{e}");
        assert!(c.resolve(Some("gemini")).is_err());
    }

    #[test]
    fn refusals_name_the_missing_piece() {
        let c = config();
        assert_eq!(c.refusal(None, None), None);
        assert_eq!(
            c.refusal(Some("openai"), Some("gpt-x")).as_deref(),
            Some("open mode needs OPENAI_API_KEY")
        );
        assert_eq!(
            c.refusal(Some("anthropic"), Some("m")).as_deref(),
            Some("open mode needs ANTHROPIC_API_KEY")
        );
        assert!(c
            .refusal(Some("local:box"), None)
            .unwrap()
            .contains("no default model"));
        assert_eq!(c.refusal(Some("local:box"), Some("llama3")), None);
        assert!(c.any_available());
        assert!(!ProviderConfig::default().any_available());
    }

    #[test]
    fn cost_is_reported_known_or_unknown_by_provider() {
        let c = config();
        let usage = serde_json::json!({"cost": 0.25});
        let cost = |id| c.resolve(Some(id)).unwrap().cost(&usage);
        assert_eq!(cost("openrouter"), Some(0.25));
        assert_eq!(cost("openai"), None);
        assert_eq!(cost("local:box"), Some(0.0));
        let source = |id| c.resolve(Some(id)).unwrap().cost_source();
        assert_eq!(source("local:box"), Some(LOCAL_COST_SOURCE));
        assert_eq!(source("openai"), None);
    }
}
