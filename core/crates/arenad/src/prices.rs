//! The list prices of `docs/prices.toml`, built into arenad, for estimating what tokens cost when
//! the provider reports no cost.

use std::sync::LazyLock;

use serde::Deserialize;

use crate::model::AgentKind;

/// One model's list price on one provider, per million tokens.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Price {
    pub provider: String,
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    /// Where the price was read.
    pub source: String,
    /// When it was read (YYYY-MM-DD).
    pub as_of: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Prices {
    #[serde(default)]
    price: Vec<Price>,
}

const BUILT_IN: &str = include_str!("../../../../docs/prices.toml");

static TABLE: LazyLock<Prices> =
    LazyLock::new(|| Prices::parse(BUILT_IN).expect("docs/prices.toml is valid: a test parses it"));

/// The built-in list prices.
pub fn table() -> &'static Prices {
    &TABLE
}

impl Prices {
    pub fn parse(text: &str) -> Result<Prices, toml::de::Error> {
        toml::from_str(text)
    }

    /// The price of `model` on `provider`, if the table has it.
    pub fn find(&self, provider: &str, model: &str) -> Option<&Price> {
        self.price
            .iter()
            .find(|p| p.provider == provider && p.model == model)
    }
}

impl Price {
    /// What `prompt` input and `completion` output tokens cost at this price.
    pub fn cost(&self, prompt: i64, completion: i64) -> f64 {
        let per = |n: i64, p: f64| n as f64 * p / 1_000_000.0;
        per(prompt, self.input_per_mtok) + per(completion, self.output_per_mtok)
    }
}

/// The provider whose prices apply to an agent: the CLI's vendor, or an open agent's provider
/// (`openrouter` when it names none).
pub fn provider_of(kind: AgentKind, provider: Option<&str>) -> Option<String> {
    match kind {
        AgentKind::ClaudeCode => Some("anthropic".into()),
        AgentKind::Codex => Some("openai".into()),
        AgentKind::Open => Some(provider.unwrap_or("openrouter").to_string()),
        AgentKind::External => provider.map(str::to_string),
        AgentKind::Http => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_table_parses_and_names_its_sources() {
        let t = table();
        assert!(!t.price.is_empty());
        for p in &t.price {
            assert!(p.source.starts_with("https://"), "{p:?}");
            assert_eq!(p.as_of.len(), 10, "{p:?}");
            assert!(p.input_per_mtok > 0.0 && p.output_per_mtok > 0.0, "{p:?}");
        }
    }

    #[test]
    fn a_price_is_per_million_tokens_of_each_kind() {
        let t = Prices::parse(
            r#"[[price]]
provider = "x"
model = "m"
input_per_mtok = 2.0
output_per_mtok = 10.0
source = "https://example.org"
as_of = "2026-11-04""#,
        )
        .unwrap();
        let p = t.find("x", "m").unwrap();
        assert_eq!(p.cost(1_000_000, 100_000), 3.0);
        assert!(t.find("x", "other").is_none() && t.find("y", "m").is_none());
        assert_eq!(
            provider_of(AgentKind::Open, None).as_deref(),
            Some("openrouter")
        );
        assert_eq!(
            provider_of(AgentKind::ClaudeCode, None).as_deref(),
            Some("anthropic")
        );
    }
}
