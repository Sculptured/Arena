//! Whether AI review is on, and with which model: `ARENA_REVIEW` (`on` or `off`; on by default
//! when the model is available) and `ARENA_REVIEW_MODEL` (`claude_code:<model>`, the default
//! `claude_code:sonnet`, or `<provider>:<model>` for one of arenad's model providers, such as
//! `openrouter:<model>`, `anthropic:<model>` or `local:<name>:<model>`). Read once.

use std::sync::OnceLock;

use serde::Serialize;

use crate::providers::{ProviderConfig, LOCAL_PREFIX};

pub const SWITCH_ENV: &str = "ARENA_REVIEW";
pub const MODEL_ENV: &str = "ARENA_REVIEW_MODEL";
pub const DEFAULT_MODEL: &str = "claude_code:sonnet";

/// The model that reviews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewerModel {
    /// Claude Code headless (`claude -p`), signed in as the CLI is.
    ClaudeCode { model: String },
    /// One of arenad's providers, called directly (not through a branch's gateway).
    Provider { provider: String, model: String },
}

impl ReviewerModel {
    /// `claude_code:<model>` or `<provider>:<model>` (`local:<name>:<model>` for a local server).
    pub fn parse(s: &str) -> Result<ReviewerModel, String> {
        let s = s.trim();
        let bad =
            || format!("{MODEL_ENV} must be claude_code:<model> or <provider>:<model>, not {s:?}");
        if let Some(rest) = s.strip_prefix(LOCAL_PREFIX) {
            let (name, model) = rest.split_once(':').ok_or_else(bad)?;
            return Self::provider(&format!("{LOCAL_PREFIX}{name}"), model).ok_or_else(bad);
        }
        let (kind, model) = s.split_once(':').ok_or_else(bad)?;
        if kind == "claude_code" {
            let model = model.trim();
            return match model.is_empty() {
                true => Err(bad()),
                false => Ok(ReviewerModel::ClaudeCode {
                    model: model.into(),
                }),
            };
        }
        Self::provider(kind, model).ok_or_else(bad)
    }

    fn provider(provider: &str, model: &str) -> Option<ReviewerModel> {
        let (provider, model) = (provider.trim(), model.trim());
        (!provider.is_empty() && !model.is_empty() && !provider.ends_with(':')).then(|| {
            ReviewerModel::Provider {
                provider: provider.into(),
                model: model.into(),
            }
        })
    }

    /// How it is named in settings and in each stored review.
    pub fn label(&self) -> String {
        match self {
            ReviewerModel::ClaudeCode { model } => format!("claude_code:{model}"),
            ReviewerModel::Provider { provider, model } => format!("{provider}:{model}"),
        }
    }
}

/// AI review's settings, as `GET /api/review/settings` reports them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewSettings {
    pub enabled: bool,
    /// The model's label (as configured, even when it can't be used).
    pub model: String,
    /// Why it is off, when it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip)]
    pub reviewer: Option<ReviewerModel>,
}

/// The settings for the switch and model given, `available` saying why a model can't run.
pub fn decide(
    switch: Option<&str>,
    model: Option<&str>,
    available: impl Fn(&ReviewerModel) -> Result<(), String>,
) -> ReviewSettings {
    let label = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    let off = |why: String| ReviewSettings {
        enabled: false,
        model: label.to_string(),
        why: Some(why),
        reviewer: None,
    };
    match switch.map(|s| s.trim().to_lowercase()).as_deref() {
        None | Some("" | "on" | "1" | "true") => {}
        Some("off" | "0" | "false") => return off(format!("{SWITCH_ENV} is off")),
        Some(other) => return off(format!("{SWITCH_ENV} must be on or off, not {other:?}")),
    }
    let reviewer = match ReviewerModel::parse(label) {
        Ok(r) => r,
        Err(why) => return off(why),
    };
    if let Err(why) = available(&reviewer) {
        return off(why);
    }
    ReviewSettings {
        enabled: true,
        model: reviewer.label(),
        why: None,
        reviewer: Some(reviewer),
    }
}

/// Why `m` can't review here: no `claude` on the PATH, or its provider is unknown or lacks a key.
pub fn availability(m: &ReviewerModel, providers: &ProviderConfig) -> Result<(), String> {
    match m {
        ReviewerModel::ClaudeCode { .. } => match crate::api::on_path("claude") {
            true => Ok(()),
            false => Err("the claude CLI is not on the PATH".into()),
        },
        ReviewerModel::Provider { provider, .. } => {
            let p = providers.resolve(Some(provider))?;
            match p.unavailable() {
                Some(u) => Err(u.why()),
                None => Ok(()),
            }
        }
    }
}

static SETTINGS: OnceLock<ReviewSettings> = OnceLock::new();

/// The settings from the environment, read on first use.
pub fn settings(providers: &ProviderConfig) -> &'static ReviewSettings {
    SETTINGS.get_or_init(|| {
        let switch = std::env::var(SWITCH_ENV).ok();
        let model = std::env::var(MODEL_ENV).ok();
        decide(switch.as_deref(), model.as_deref(), |m| {
            availability(m, providers)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yes(_: &ReviewerModel) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn models_parse_from_their_labels() {
        let cc = ReviewerModel::parse("claude_code:sonnet").unwrap();
        assert_eq!(
            cc,
            ReviewerModel::ClaudeCode {
                model: "sonnet".into()
            }
        );
        let or = ReviewerModel::parse("openrouter:deepseek/deepseek-chat-v3.1").unwrap();
        assert_eq!(or.label(), "openrouter:deepseek/deepseek-chat-v3.1");
        let local = ReviewerModel::parse("local:ollama:llama3.3:70b").unwrap();
        assert_eq!(
            local,
            ReviewerModel::Provider {
                provider: "local:ollama".into(),
                model: "llama3.3:70b".into()
            }
        );
        for bad in ["sonnet", "claude_code:", ":m", "local:ollama", "local::m"] {
            assert!(ReviewerModel::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn on_by_default_when_the_model_can_run_and_off_means_off() {
        let s = decide(None, None, yes);
        assert!(s.enabled);
        assert_eq!(s.model, DEFAULT_MODEL);
        let s = decide(Some("off"), None, yes);
        assert!(!s.enabled && s.reviewer.is_none());
        assert_eq!(s.why.as_deref(), Some("ARENA_REVIEW is off"));
        // On, but the model can't run: off, saying why.
        let s = decide(Some("on"), Some("anthropic:claude-x"), |_| {
            Err("ANTHROPIC_API_KEY is not set".into())
        });
        assert!(!s.enabled);
        assert_eq!(s.why.as_deref(), Some("ANTHROPIC_API_KEY is not set"));
        assert_eq!(s.model, "anthropic:claude-x");
        assert!(!decide(Some("maybe"), None, yes).enabled);
        assert!(!decide(None, Some("nonsense"), yes).enabled);
    }
}
