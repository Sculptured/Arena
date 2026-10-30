use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use axum::Router;
use world::snapshot::Worlds;

use crate::{exec::BranchCtx, store::Store};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
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
    crate::mcp::router().with_state(state)
}
