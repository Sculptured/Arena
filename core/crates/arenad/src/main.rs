//! The arena service (`cargo run -p arenad` from `arena/core`).
//!
//! Environment: `ARENA_PG_URL`, `ARENA_PORT` (4510), `OPENROUTER_API_KEY`,
//! `ARENA_UI_DIST` (serve the built UI at `/`),
//! `ARENA_MAX_DB_BYTES` (default 5 GiB: new runs and forks are refused above it),
//! `ARENA_BENCHMARK` (default `retail`), and where the parts are:
//! - `ARENA_PACKAGES`: `:`-separated folders whose subfolders hold `arena-package.toml`
//!   (default `<arena>/integrations:<arena>/benchmarks`);
//! - `ARENA_BIN_DIR`: substituted for `${ARENA_BIN_DIR}` in package commands
//!   (default `<arena>/core/target/release`; build with `cargo build --release` in `<arena>`);
//! - `ARENA_SAVES`: save storage (default `<arena>/core/saves`).
//!
//! `<arena>` is the folder above `core`, fixed when arenad is built (from `CARGO_MANIFEST_DIR`,
//! which is `<arena>/core/crates/arenad`), so the defaults hold wherever arenad is started from.

use std::{path::PathBuf, sync::Arc};

use arenad::{
    app::{self, AppState, Config},
    pg::Admin,
    store::Store,
};
use parts::{Hub, Registry};

fn arena_dir() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    here.canonicalize().unwrap_or(here)
}

fn env_path(key: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or(default)
}

/// The hub over `ARENA_PACKAGES`, `ARENA_BIN_DIR` and `ARENA_SAVES`; programs get `ARENA_PG_URL`.
fn hub(pg_url: &str) -> anyhow::Result<Hub> {
    let arena = arena_dir();
    let roots: Vec<PathBuf> = match std::env::var("ARENA_PACKAGES") {
        Ok(p) if !p.is_empty() => std::env::split_paths(&p).collect(),
        _ => vec![arena.join("integrations"), arena.join("benchmarks")],
    };
    let bin_dir = env_path("ARENA_BIN_DIR", arena.join("core/target/release"));
    let saves = env_path("ARENA_SAVES", arena.join("core/saves"));
    std::fs::create_dir_all(&saves)?;
    let registry = Registry::scan(&roots)?;
    Ok(Hub::new(registry, bin_dir, saves)
        .with_env(vec![("ARENA_PG_URL".into(), pg_url.to_string())]))
}

/// The store and the hub.
async fn app_state(url: &str, port: u16) -> anyhow::Result<Arc<AppState>> {
    let admin = Admin::connect(url).await?;
    let store = Arc::new(Store::open(&admin).await?);
    let hub = Arc::new(hub(url)?);
    let mut config = Config::new();
    config.self_base = format!("http://127.0.0.1:{port}");
    Ok(Arc::new(AppState::new(store, hub, config)))
}

/// `ARENA_PORT`, or 4510.
fn port_from_env() -> anyhow::Result<u16> {
    match std::env::var("ARENA_PORT") {
        Ok(p) => p
            .parse()
            .map_err(|_| anyhow::anyhow!("ARENA_PORT {p:?} is not a port number")),
        Err(_) => Ok(4510),
    }
}

/// The modes this machine can run, for the startup line.
fn available_modes(state: &AppState) -> Vec<&'static str> {
    [
        ("claude-code", arenad::api::on_path("claude")),
        ("codex", arenad::api::on_path("codex")),
        ("open", state.config.open_available()),
    ]
    .into_iter()
    .filter(|m| m.1)
    .map(|m| m.0)
    .collect()
}

/// Ends the SSE streams on ctrl-c so graceful shutdown can finish; exits anyway after 5 s.
async fn shutdown_on_ctrl_c(state: Arc<AppState>) {
    let _ = tokio::signal::ctrl_c().await;
    let _ = state.shutdown.send(true);
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        std::process::exit(0);
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string());
    let port = port_from_env()?;
    let ui_dist = std::env::var("ARENA_UI_DIST")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);

    let state = app_state(&url, port).await?;
    // Fail at startup if the benchmark can't be described (package missing or not built).
    let bench = state
        .bench()
        .await
        .map_err(|e| anyhow::anyhow!("benchmark {}: {e:#}", state.benchmark))?;
    // Before any run can save: drop the saves no event names (left by crashes and aborted tests).
    arenad::gc::run_startup_gc(&state.store, &state.hub, &state.benchmark).await?;
    app::install_default_drivers(&state);

    let mut router = app::router(state.clone());
    if let Some(dist) = &ui_dist {
        router = arenad::api::with_ui(router, dist.clone());
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!(
        "arenad: listening on http://127.0.0.1:{port} (benchmark: {} {}; modes: {}; ui: {})",
        bench.name,
        bench.version,
        available_modes(&state).join(", "),
        ui_dist.map_or("api only".to_string(), |d| d.display().to_string())
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_on_ctrl_c(state))
        .await?;
    Ok(())
}
