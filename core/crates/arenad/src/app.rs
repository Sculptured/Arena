use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use axum::Router;
use world::snapshot::Worlds;

use crate::{exec::BranchCtx, store::Store};

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
        self.upstream_base.contains("openrouter.ai")
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
}

impl AppState {
    pub fn new(store: Arc<Store>, worlds: Arc<Worlds>, config: Config) -> AppState {
        AppState { store, worlds, live: RwLock::new(HashMap::new()), config }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    crate::mcp::router().merge(crate::gateway::router()).with_state(state)
}
