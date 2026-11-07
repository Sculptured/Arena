//! The factory benchmark's world, whole: ERPNext, Twenty, the hub, the stand-in plant and
//! Sculptured, with the benchmark's own team on a real model (see `support/live.rs`). Whatever the
//! team does, Arena must record every step, keep the gates (a change no one proposed is never
//! acted on), and score the end state with every named check. Which end states pass which check
//! is tested on the benchmark's own checks (`benchmarks/factory/src/checks.rs`).
//!
//! `live_`: it starts an ERPNext stack and a Twenty server of its own, and needs the Sculptured
//! program, the factory example (not public yet) and a model, so it says why and passes without
//! them. Everything it makes is removed when it ends. Run:
//! `cargo test -p arenad --test factory -- --ignored live_ --nocapture`.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/live.rs"]
mod live;
#[path = "support/live_world.rs"]
mod live_world;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{process::Command, sync::Arc};

use arenad::app::{AppState, Config};
use live::Live;
use live_world::{path_with_sculptured, walk_for, Stack};
use packages::{arena, bin_dir, package_roots};
use parts::{Hub, Registry};
use pg::Cleanup;
use serde_json::json;

const TASK: &str = "press-a1-bearing";

/// Why this machine can't run the world, if it can't.
fn missing() -> Option<String> {
    let example = arena().join("../Sculptured-Examples/factory/data/ridgeline.json");
    if !example.exists() {
        return Some(format!("no factory example at {}", example.display()));
    }
    let mut version = Command::new("sculptured");
    let found = version
        .arg("--version")
        .env("PATH", path_with_sculptured())
        .output();
    found.is_err().then(|| "no sculptured program".to_string())
}

fn build() {
    let st = Command::new("cargo")
        .args([
            "build",
            "-q",
            "-p",
            "arena-factory",
            "-p",
            "arena-erpnext",
            "-p",
            "arena-sculptured",
            "-p",
            "arena-twenty-crm",
        ])
        .current_dir(arena())
        .status()
        .unwrap();
    assert!(st.success(), "building the package binaries failed");
}

async fn app(hub: &Arc<Hub>, tag: &str) -> (Arc<AppState>, Cleanup) {
    let (store, clean) = app_support::meta_store("factory", tag).await;
    let mut config = Config::new();
    let (listener, base) = app_support::bind().await;
    config.self_base = base;
    let state = AppState::new(store, hub.clone(), config).with_benchmark("factory");
    let state = Arc::new(state);
    live::install(&state);
    app_support::serve(listener, &state);
    (state, clean)
}

/// Runs the team to its end (up to `polls` × 200 ms); asserts what Arena owes any run of it, and
/// prints the checks and metrics (measured time, arena's overhead, the estimate in production).
async fn run_and_report(hub: &Arc<Hub>, live: &Live, polls: usize) {
    let bench = hub.bench_describe("factory").await.unwrap();
    let (state, _clean) = app(hub, "live").await;
    let team = live.bench_team(bench.team.clone().unwrap());
    let (b, check, events) = walk_for(&state, TASK, team, polls).await;
    let m = arenad::metrics::metrics(&b, &events, arenad::prices::table(), chrono::Utc::now());
    let report = json!({"status": b.status, "error": b.error, "check": check, "metrics": m});
    println!("REPORT {}", serde_json::to_string_pretty(&report).unwrap());
    live::assert_recorded(&b, &events, live);
    live_world::assert_scored(&b, &check);
    live_world::assert_gates_kept(&bench, &events);
}

/// The incident with the benchmark's own team. For a run whose parts are not the defaults, set by
/// the environment: `ARENA_RUN_FACTORY` (e.g. `--backend isaac --plant-config ...`),
/// `ARENA_RUN_ISAAC_SIM` (the plant on another machine) and `ARENA_CLOCK_SPEED` (default 20). It
/// waits up to an hour, since Isaac Sim starts and steps far slower than the stand-in.
#[test]
#[ignore = "live: runs a real model on a whole world (see the module doc)"]
fn live_factory_team_runs_the_incident_and_arena_scores_it() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    let Some(live) = Live::any("live_factory_team_runs_the_incident_and_arena_scores_it") else {
        return;
    };
    build();
    if std::env::var_os("ARENA_CLOCK_SPEED").is_none() {
        // SAFETY: set before the runtime or any thread that reads the environment starts.
        unsafe { std::env::set_var("ARENA_CLOCK_SPEED", "20") };
    }
    let stack = Stack::new("t13");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run_and_report(&hub, &live, 18_000));
}
