//! Which providers this arenad can use, read once from the environment at start.
//!
//! - `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`;
//! - `ARENA_LOCAL_MODELS="name=url[,name=url…]"`: OpenAI-compatible servers (Ollama, vLLM, LM
//!   Studio, llama.cpp server), each the provider `local:<name>`;
//! - `ARENA_LOCAL_KEY_<NAME>`: an optional key for `local:<name>` (`<NAME>` upper-cased, `-` as
//!   `_`).
//!
//! Keys are held as [`Secret`]s: never logged, printed, serialized or recorded.

use super::{CUSTOM_LOCAL, LOCAL_PREFIX, OPENROUTER};

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";
pub const LOCAL_MODELS_ENV: &str = "ARENA_LOCAL_MODELS";
pub const LOCAL_KEY_PREFIX: &str = "ARENA_LOCAL_KEY_";

/// A credential. Its `Debug` is `<redacted>` and it has no `Display` or `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Secret {
        Secret(key.into())
    }

    /// The key itself, for the one header that carries it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where a provider answers, and the key it takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub base: String,
    pub key: Option<Secret>,
}

impl Endpoint {
    pub fn new(base: &str, key: Option<Secret>) -> Endpoint {
        Endpoint {
            base: base.to_string(),
            key,
        }
    }
}

/// An OpenAI-compatible server configured as `local:<name>`.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEndpoint {
    pub name: String,
    pub endpoint: Endpoint,
}

/// The providers' endpoints and keys.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderConfig {
    pub openrouter: Endpoint,
    pub openai: Endpoint,
    pub anthropic: Endpoint,
    pub locals: Vec<LocalEndpoint>,
    /// The provider of agents that name none: `openrouter`, or `local:custom` after
    /// [`ProviderConfig::set_upstream`].
    pub default: String,
}

impl Default for ProviderConfig {
    /// No keys and no local servers: only the public endpoints, none of them usable.
    fn default() -> ProviderConfig {
        ProviderConfig {
            openrouter: Endpoint::new(OPENROUTER_BASE, None),
            openai: Endpoint::new(OPENAI_BASE, None),
            anthropic: Endpoint::new(ANTHROPIC_BASE, None),
            locals: vec![],
            default: OPENROUTER.to_string(),
        }
    }
}

/// True only for `https` URLs whose host is exactly `openrouter.ai`.
pub fn is_openrouter_url(base: &str) -> bool {
    reqwest::Url::parse(base)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some("openrouter.ai"))
}

/// The name of the env var holding the key of `local:<name>`.
pub fn local_key_env(name: &str) -> String {
    format!(
        "{LOCAL_KEY_PREFIX}{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

fn valid_local_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn valid_base(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

impl ProviderConfig {
    /// The configuration in the process environment; problems with `ARENA_LOCAL_MODELS` entries
    /// are printed (by name, never with their URL) and the entries skipped.
    pub fn from_env() -> ProviderConfig {
        let (config, problems) = ProviderConfig::from_lookup(|k| std::env::var(k).ok());
        for p in problems {
            eprintln!("arenad: {LOCAL_MODELS_ENV}: {p}");
        }
        config
    }

    /// The configuration `get` describes (it reads one env var), and the problems found.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> (ProviderConfig, Vec<String>) {
        let key = |k: &str| get(k).filter(|v| !v.trim().is_empty()).map(Secret::new);
        let mut config = ProviderConfig::default();
        config.openrouter.key = key("OPENROUTER_API_KEY");
        config.openai.key = key("OPENAI_API_KEY");
        config.anthropic.key = key("ANTHROPIC_API_KEY");
        let list = get(LOCAL_MODELS_ENV).unwrap_or_default();
        let (locals, problems) = parse_locals(&list, |name| key(&local_key_env(name)));
        config.locals = locals;
        (config, problems)
    }

    /// Nothing from the environment: only the OpenAI-compatible `base`, as `local:custom`, for
    /// agents that name no provider (see [`ProviderConfig::set_upstream`]).
    pub fn with_upstream(base: &str) -> ProviderConfig {
        let mut config = ProviderConfig::default();
        config.set_upstream(base);
        config
    }

    /// The old single-upstream override: agents that name no provider go to the
    /// OpenAI-compatible `base`, as the local provider `local:custom`, which gets no key. A
    /// `base` that is OpenRouter itself changes nothing.
    pub fn set_upstream(&mut self, base: &str) {
        if is_openrouter_url(base) {
            return;
        }
        self.locals.retain(|l| l.name != CUSTOM_LOCAL);
        self.locals.push(LocalEndpoint {
            name: CUSTOM_LOCAL.to_string(),
            endpoint: Endpoint::new(base, None),
        });
        self.default = format!("{LOCAL_PREFIX}{CUSTOM_LOCAL}");
    }

    pub fn local(&self, name: &str) -> Option<&LocalEndpoint> {
        self.locals.iter().find(|l| l.name == name)
    }
}

/// The local servers in `list` (`name=url,…`), each with the key `key_of(name)` finds; and why
/// any entry was skipped.
fn parse_locals(
    list: &str,
    key_of: impl Fn(&str) -> Option<Secret>,
) -> (Vec<LocalEndpoint>, Vec<String>) {
    let mut locals: Vec<LocalEndpoint> = vec![];
    let mut problems = vec![];
    for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        match parse_local(entry, &locals) {
            Ok((name, base)) => locals.push(LocalEndpoint {
                endpoint: Endpoint::new(base, key_of(name)),
                name: name.to_string(),
            }),
            Err(p) => problems.push(p),
        }
    }
    (locals, problems)
}

/// One `name=url` entry, checked against the entries before it.
fn parse_local<'a>(entry: &'a str, seen: &[LocalEndpoint]) -> Result<(&'a str, &'a str), String> {
    let Some((name, base)) = entry.split_once('=') else {
        return Err("an entry is not name=url; it was skipped".into());
    };
    let (name, base) = (name.trim(), base.trim());
    if !valid_local_name(name) {
        return Err(format!("{name:?} must match ^[a-z0-9_-]+$; it was skipped"));
    }
    if !valid_base(base) {
        return Err(format!("{name}: the URL is not http(s); it was skipped"));
    }
    if seen.iter().any(|l| l.name == name) {
        return Err(format!("{name} is listed twice; the first one is used"));
    }
    Ok((name, base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config_of(vars: &[(&str, &str)]) -> (ProviderConfig, Vec<String>) {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ProviderConfig::from_lookup(|k| vars.get(k).cloned())
    }

    #[test]
    fn keys_and_local_servers_come_from_the_environment() {
        let (c, problems) = config_of(&[
            ("OPENAI_API_KEY", "sk-openai"),
            ("ANTHROPIC_API_KEY", " "),
            (
                LOCAL_MODELS_ENV,
                " ollama=http://127.0.0.1:11434/v1 , lm-studio=http://127.0.0.1:1234/v1,",
            ),
            ("ARENA_LOCAL_KEY_LM_STUDIO", "sk-local"),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.openrouter.key, None);
        assert_eq!(c.openai.key, Some(Secret::new("sk-openai")));
        assert_eq!(c.anthropic.key, None);
        let names: Vec<_> = c.locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["ollama", "lm-studio"]);
        assert_eq!(c.locals[0].endpoint.base, "http://127.0.0.1:11434/v1");
        assert_eq!(c.locals[0].endpoint.key, None);
        assert_eq!(c.locals[1].endpoint.key, Some(Secret::new("sk-local")));
        assert_eq!(c.default, "openrouter");
    }

    #[test]
    fn bad_local_entries_are_skipped_without_their_url() {
        let (c, problems) = config_of(&[(
            LOCAL_MODELS_ENV,
            "noequals,Bad=http://a,ftp=ftp://user:pw@a,x=http://a,x=http://b",
        )]);
        assert_eq!(c.locals.len(), 1);
        assert_eq!(c.locals[0].endpoint.base, "http://a");
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems.iter().all(|p| !p.contains("pw")));
    }

    #[test]
    fn keys_are_redacted_in_debug_output() {
        let (c, _) = config_of(&[
            ("OPENROUTER_API_KEY", "sk-or-secret"),
            (LOCAL_MODELS_ENV, "a=http://a"),
            ("ARENA_LOCAL_KEY_A", "sk-local-secret"),
        ]);
        let text = format!("{c:?}");
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn the_upstream_override_becomes_local_custom() {
        let mut c = ProviderConfig::default();
        c.set_upstream(OPENROUTER_BASE);
        assert_eq!((c.default.as_str(), c.locals.len()), ("openrouter", 0));
        c.set_upstream("http://127.0.0.1:9/v1");
        assert_eq!(c.default, "local:custom");
        assert_eq!(
            c.local("custom").unwrap().endpoint.base,
            "http://127.0.0.1:9/v1"
        );
        assert_eq!(local_key_env("my-box"), "ARENA_LOCAL_KEY_MY_BOX");
    }

    #[test]
    fn openrouter_detection_is_exact() {
        assert!(is_openrouter_url("https://openrouter.ai/api/v1"));
        assert!(!is_openrouter_url("https://openrouter.ai.evil.com/api/v1"));
        assert!(!is_openrouter_url("http://openrouter.ai"));
        assert!(!is_openrouter_url("https://evil.com/openrouter.ai"));
        assert!(!is_openrouter_url("not a url"));
    }
}
