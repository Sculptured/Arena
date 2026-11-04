use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

use axum::Router;
use parts::{proto::BenchDescribe, Hub};

use crate::{
    agents::{claude::ClaudeDriver, codex::CodexDriver, open::OpenDriver, Driver},
    exec::BranchCtx,
    model::AgentKind,
    providers::ProviderConfig,
    store::Store,
};

pub type Drivers = HashMap<AgentKind, Arc<dyn Driver>>;

#[derive(Debug, Clone)]
pub struct Config {
    /// The model providers open agents can use, and their keys (redacted in `Debug`).
    pub providers: ProviderConfig,
    /// This service's own base URL (e.g. `http://127.0.0.1:8787`), used by the open agent loop.
    pub self_base: String,
    /// New runs and forks are refused (507) once the arena's saves use at least this many bytes.
    pub max_db_bytes: u64,
    /// The wait before an open agent retries a model call its provider turned down as
    /// overloaded or rate-limited, doubling for each further retry (unless it says how long).
    pub retry_base: Duration,
}

pub const DEFAULT_MAX_DB_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const DEFAULT_RETRY_BASE: Duration = Duration::from_secs(1);

pub use crate::providers::{is_openrouter_url, OPENROUTER_BASE};

impl Default for Config {
    fn default() -> Config {
        Config::new()
    }
}

impl Config {
    /// Defaults plus the providers' keys and local servers from the environment.
    pub fn new() -> Config {
        Config {
            providers: ProviderConfig::from_env(),
            self_base: String::new(),
            max_db_bytes: std::env::var("ARENA_MAX_DB_BYTES")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(DEFAULT_MAX_DB_BYTES),
            retry_base: DEFAULT_RETRY_BASE,
        }
    }

    /// Open-mode agents can run: some provider has what it needs.
    pub fn open_available(&self) -> bool {
        self.providers.any_available()
    }
}

pub struct AppState {
    pub store: Arc<Store>,
    /// The installed integrations and benchmarks, their programs started on first use.
    pub hub: Arc<Hub>,
    /// The benchmark new runs use: env `ARENA_BENCHMARK`, else the only one installed; empty
    /// when neither names one.
    pub benchmark: String,
    /// Branches currently running, by branch id.
    pub live: RwLock<HashMap<String, Arc<BranchCtx>>>,
    pub config: Config,
    /// Shared HTTP client (gateway upstream and open agent loop).
    pub http: reqwest::Client,
    /// Set once, after the `Arc<AppState>` exists (the open driver holds a `Weak` to it).
    pub drivers: OnceLock<Drivers>,
    /// Set to true when shutdown starts; SSE streams end on it.
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// Held for writing by a cleanup (`POST /api/gc`) and for reading while a run or fork
    /// starts, so a cleanup never drops saves a starting branch is about to name.
    pub cleanup: Arc<tokio::sync::RwLock<()>>,
}

impl AppState {
    pub fn new(store: Arc<Store>, hub: Arc<Hub>, config: Config) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("http client");
        let benchmark = std::env::var("ARENA_BENCHMARK")
            .ok()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| only_benchmark(&hub));
        AppState {
            store,
            hub,
            benchmark,
            live: RwLock::new(HashMap::new()),
            config,
            http,
            drivers: OnceLock::new(),
            shutdown: tokio::sync::watch::channel(false).0,
            cleanup: Arc::default(),
        }
    }

    /// Uses `name` as the benchmark of new runs.
    pub fn with_benchmark(mut self, name: &str) -> AppState {
        self.benchmark = name.to_string();
        self
    }
}

impl AppState {
    /// The `describe` of the benchmark new runs use: its world, tasks, instructions and tools.
    pub async fn bench(&self) -> anyhow::Result<BenchDescribe> {
        if self.benchmark.is_empty() {
            anyhow::bail!(no_benchmark_chosen(&self.hub));
        }
        self.hub.bench_describe(&self.benchmark).await
    }

    /// Installs the driver map; false if one was already installed.
    pub fn install_drivers(&self, drivers: Drivers) -> bool {
        self.drivers.set(drivers).is_ok()
    }
}

/// Why no benchmark is chosen: none is installed, or several are and `ARENA_BENCHMARK` names none.
fn no_benchmark_chosen(hub: &Hub) -> String {
    let installed: Vec<&str> = hub.registry().benchmarks().map(|m| &*m.name).collect();
    if installed.is_empty() {
        return "no benchmark is installed: no folder in ARENA_PACKAGES holds a benchmark package"
            .into();
    }
    format!(
        "several benchmarks are installed: set ARENA_BENCHMARK to one of: {}",
        installed.join(", ")
    )
}

/// The name of the one installed benchmark; empty when there are none or several.
fn only_benchmark(hub: &Hub) -> String {
    let benchmarks = hub.registry().benchmarks().filter(|m| !m.opt_in);
    let mut names = benchmarks.map(|m| m.name.clone());
    match (names.next(), names.next()) {
        (Some(one), None) => one,
        _ => String::new(),
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

#[cfg(test)]
mod tests {
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
