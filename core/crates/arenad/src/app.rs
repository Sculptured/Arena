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
    /// New runs and forks are refused (507) once the arena's databases use at least this many bytes.
    pub max_db_bytes: u64,
}

pub const DEFAULT_MAX_DB_BYTES: u64 = 5 * 1024 * 1024 * 1024;

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

/// True only for `https` URLs whose host is exactly `openrouter.ai`.
pub fn is_openrouter_url(base: &str) -> bool {
    reqwest::Url::parse(base)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some("openrouter.ai"))
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("upstream_base", &self.upstream_base)
            .field(
                "openrouter_key",
                &self.openrouter_key.as_ref().map(|_| "<redacted>"),
            )
            .field("self_base", &self.self_base)
            .field("max_db_bytes", &self.max_db_bytes)
            .finish()
    }
}

impl Config {
    /// Defaults plus `OPENROUTER_API_KEY` from the environment.
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            upstream_base: OPENROUTER_BASE.to_string(),
            openrouter_key: std::env::var("OPENROUTER_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            self_base: String::new(),
            max_db_bytes: std::env::var("ARENA_MAX_DB_BYTES")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(DEFAULT_MAX_DB_BYTES),
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
    /// Set to true when shutdown starts; SSE streams end on it.
    pub shutdown: tokio::sync::watch::Sender<bool>,
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
        AppState {
            store,
            worlds,
            live: RwLock::new(HashMap::new()),
            config,
            http,
            drivers: OnceLock::new(),
            tasks: OnceLock::new(),
            shutdown: tokio::sync::watch::channel(false).0,
            dumps: Default::default(),
        }
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
    m.insert(
        AgentKind::ClaudeCode,
        Arc::new(ClaudeDriver::new(base.clone())),
    );
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

    #[test]
    fn loopback_hosts() {
        use super::is_loopback_host as ok;
        for h in [
            "127.0.0.1",
            "127.0.0.1:4510",
            "localhost",
            "LOCALHOST:80",
            "[::1]",
            "[::1]:4510",
        ] {
            assert!(ok(h), "{h}");
        }
        for h in [
            "evil.example",
            "127.0.0.1.evil.example",
            "localhost.evil.com:80",
            "127.0.0.1:abc",
            "[::2]",
            "",
            "::1",
        ] {
            assert!(!ok(h), "{h}");
        }
    }
}

/// True when `host` (a Host header value) names this machine's loopback: `127.0.0.1`, `localhost`
/// or `[::1]`, with an optional port.
fn is_loopback_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((ip, "")) => return ip == "::1",
            Some((ip, port)) => {
                return ip == "::1"
                    && port
                        .strip_prefix(':')
                        .is_some_and(|p| p.parse::<u16>().is_ok())
            }
            None => return false,
        },
        None => match host.split_once(':') {
            Some((n, port)) if port.parse::<u16>().is_ok() => n,
            Some(_) => return false,
            None => host,
        },
    };
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost")
}

/// DNS-rebinding guard: a web page on another origin can resolve its own name to 127.0.0.1, so the
/// browser would send that name in `Host`. Only loopback names reach the API.
async fn loopback_host_only(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ok = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(is_loopback_host);
    if !ok {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "forbidden host"})),
        )
            .into_response();
    }
    next.run(req).await
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = crate::api::router().layer(axum::middleware::from_fn(loopback_host_only));
    crate::mcp::router()
        .merge(crate::gateway::router())
        .merge(api)
        .with_state(state)
}
