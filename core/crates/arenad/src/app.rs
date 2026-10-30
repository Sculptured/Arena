use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};

use axum::Router;
use world::snapshot::Worlds;

use crate::{
    agents::{claude::ClaudeDriver, codex::CodexDriver, open::OpenDriver, Driver},
    exec::BranchCtx,
    model::AgentKind,
    store::Store,
    tasks::Tasks,
};

pub type Drivers = HashMap<AgentKind, Arc<dyn Driver>>;

#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// OpenAI-compatible base the gateway forwards to.
    pub upstream_base: String,
    /// Never logged or recorded.
    pub openrouter_key: Option<String>,
    /// This service's own base URL (e.g. `http://127.0.0.1:8787`), used by the open agent loop.
    pub self_base: String,
}

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

/// True only for `https` URLs whose host is exactly `openrouter.ai`.
pub fn is_openrouter_url(base: &str) -> bool {
    reqwest::Url::parse(base).is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some("openrouter.ai"))
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("upstream_base", &self.upstream_base)
            .field("openrouter_key", &self.openrouter_key.as_ref().map(|_| "<redacted>"))
            .field("self_base", &self.self_base)
            .finish()
    }
}

impl Config {
    /// Defaults plus `OPENROUTER_API_KEY` from the environment.
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            upstream_base: OPENROUTER_BASE.to_string(),
            openrouter_key: std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.is_empty()),
            self_base: String::new(),
        }
    }

    pub fn upstream_is_openrouter(&self) -> bool {
        is_openrouter_url(&self.upstream_base)
    }

    /// Open-mode agents can run: a key is set, or the upstream is not OpenRouter.
    pub fn open_available(&self) -> bool {
        self.openrouter_key.is_some() || !self.upstream_is_openrouter()
    }
}

pub struct AppState {
    pub store: Arc<Store>,
    pub worlds: Arc<Worlds>,
    /// Branches currently running, by branch id.
    pub live: RwLock<HashMap<String, Arc<BranchCtx>>>,
    pub config: Config,
    /// Shared HTTP client (gateway upstream and open agent loop).
    pub http: reqwest::Client,
    /// Set once, after the `Arc<AppState>` exists (the open driver holds a `Weak` to it).
    pub drivers: OnceLock<Drivers>,
    tasks: OnceLock<Tasks>,
    /// Final database dumps of finished branches, by branch id (for compare).
    pub dumps: std::sync::Mutex<HashMap<String, serde_json::Value>>,
}

impl AppState {
    pub fn new(store: Arc<Store>, worlds: Arc<Worlds>, config: Config) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("http client");
        AppState { store, worlds, live: RwLock::new(HashMap::new()), config, http, drivers: OnceLock::new(), tasks: OnceLock::new(), dumps: Default::default() }
    }
}

impl AppState {
    /// The tasks under `config.data_dir`, loaded on first use.
    pub fn tasks(&self) -> anyhow::Result<&Tasks> {
        if let Some(t) = self.tasks.get() {
            return Ok(t);
        }
        let loaded = Tasks::load(&self.config.data_dir)?;
        Ok(self.tasks.get_or_init(|| loaded))
    }

    /// Installs the driver map; false if one was already installed.
    pub fn install_drivers(&self, drivers: Drivers) -> bool {
        self.drivers.set(drivers).is_ok()
    }
}

/// The real drivers for every agent kind. Call once, after `config.self_base` is final.
pub fn install_default_drivers(state: &Arc<AppState>) -> bool {
    let base = state.config.self_base.clone();
    let mut m: Drivers = HashMap::new();
    m.insert(AgentKind::Open, Arc::new(OpenDriver::new(state.clone())));
    m.insert(AgentKind::ClaudeCode, Arc::new(ClaudeDriver::new(base.clone())));
    m.insert(AgentKind::Codex, Arc::new(CodexDriver::new(base)));
    state.install_drivers(m)
}

#[cfg(test)]
mod tests {
    use super::is_openrouter_url;

    #[test]
    fn openrouter_detection_is_exact() {
        assert!(is_openrouter_url("https://openrouter.ai/api/v1"));
        assert!(!is_openrouter_url("https://openrouter.ai.evil.com/api/v1"));
        assert!(!is_openrouter_url("http://openrouter.ai"));
        assert!(!is_openrouter_url("https://evil.com/openrouter.ai"));
        assert!(!is_openrouter_url("not a url"));
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    crate::mcp::router().merge(crate::gateway::router()).merge(crate::api::router()).with_state(state)
}
