//! The arena service. Run it from `arena/core` (`cargo run -p arenad`): `ARENA_DATA` defaults to
//! `../data/retail`, relative to the current directory.
//!
//! Environment: `ARENA_PG_URL`, `ARENA_DATA`, `ARENA_PORT` (4510), `OPENROUTER_API_KEY`,
//! `ARENA_UI_DIST` (serve the built UI at `/`),
//! `ARENA_MAX_DB_BYTES` (default 5 GiB: new runs and forks are refused above it).

use std::{path::PathBuf, sync::Arc};

use arenad::{
    app::{self, AppState, Config},
    store::Store,
};
use world::{pg::Admin, snapshot::Worlds};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string());
    let port: u16 = match std::env::var("ARENA_PORT") {
        Ok(p) => p.parse().map_err(|_| anyhow::anyhow!("ARENA_PORT {p:?} is not a port number"))?,
        Err(_) => 4510,
    };
    let data_dir = PathBuf::from(std::env::var("ARENA_DATA").unwrap_or_else(|_| "../data/retail".to_string()));
    let ui_dist = std::env::var("ARENA_UI_DIST").ok().filter(|p| !p.is_empty()).map(PathBuf::from);

    let admin = Admin::connect(&url).await?;
    let store = Arc::new(Store::open(&admin).await?);
    world::schema::load_base(&admin, &data_dir).await?;
    let worlds = Arc::new(Worlds::new(admin));
    // Before anything can create a database: drop orphans from crashes and aborted tests.
    arenad::gc::run_startup_gc(&store, &worlds).await?;
    let mut config = Config::new(data_dir);
    config.self_base = format!("http://127.0.0.1:{port}");
    let state = Arc::new(AppState::new(store, worlds, config));
    state.tasks()?; // fail at startup if the task data is missing
    app::install_default_drivers(&state);

    let mut router = app::router(state.clone());
    if let Some(dist) = &ui_dist {
        router = arenad::api::with_ui(router, dist.clone());
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let modes = [
        ("claude-code", arenad::api::on_path("claude")),
        ("codex", arenad::api::on_path("codex")),
        ("open", state.config.open_available()),
    ];
    let modes: Vec<&str> = modes.iter().filter(|m| m.1).map(|m| m.0).collect();
    println!(
        "arenad: listening on http://127.0.0.1:{port} (modes: {}; ui: {})",
        modes.join(", "),
        ui_dist.map_or("api only".to_string(), |d| d.display().to_string())
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            // End the SSE streams so graceful shutdown can finish; exit anyway after 5 s.
            let _ = state.shutdown.send(true);
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                std::process::exit(0);
            });
        })
        .await?;
    Ok(())
}
