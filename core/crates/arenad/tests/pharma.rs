//! The pharma benchmark's world, whole: ERPNext, Twenty, the simulated LIMS and carriers and
//! Sculptured's approvals, with the benchmark's own team on a real model (see
//! `support/live.rs`); every change is decided by the simulated QA head or supply chain director
//! through the world's links. Whatever the team does, Arena must record every step, keep the
//! gates and score the end state with every named check; a disruption must show in the twin's
//! views. Which end states pass which check (the gold paths, a release without approval, the
//! held batch shipped) is tested on the benchmark's own checks
//! (`benchmarks/pharma/src/checks/tests.rs`).
//!
//! `live_`: it starts an ERPNext stack and a Twenty server of its own and needs the Sculptured
//! program (`ARENA_SCULPTURED_BINARY`: one recent enough to serve the changes route) and a model;
//! everything it makes is removed when it ends. Run:
//! `cargo test -p arenad --test pharma -- --ignored live_ --nocapture`.
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

use arenad::{
    app::{AppState, Config},
    model::*,
};
use live::Live;
use live_world::{path_with_sculptured, walk_for, walk_in, Stack};
use packages::{arena, bin_dir, package_roots};
use parts::{Hub, Registry};
use pg::Cleanup;
use serde_json::{json, Value};

const TASK: &str = "cvx-2611-em-excursion";

async fn app(hub: &Arc<Hub>, tag: &str) -> (Arc<AppState>, Cleanup) {
    let (store, clean) = app_support::meta_store("pharma", tag).await;
    let mut config = Config::new();
    let (listener, base) = app_support::bind().await;
    config.self_base = base;
    let state = AppState::new(store, hub.clone(), config).with_benchmark("pharma");
    let state = Arc::new(state);
    live::install(&state);
    app_support::serve(listener, &state);
    (state, clean)
}

/// The base incident with the benchmark's own team: recorded, gated and scored.
async fn assert_team_run(hub: &Arc<Hub>, live: &Live) {
    let bench = hub.bench_describe("pharma").await.unwrap();
    let (state, _clean) = app(hub, "team").await;
    let team = live.bench_team(bench.team.clone().unwrap());
    let (b, check, evs) = walk_for(&state, TASK, team, 18_000).await;
    println!("{TASK}: {:?} {:?} {check:?}", b.status, b.error);
    live::assert_recorded(&b, &evs, live);
    live_world::assert_scored(&b, &check);
    live_world::assert_gates_kept(&bench, &evs);
}

/// Tundra full after its first quote.
fn courier_full(bench: &parts::proto::BenchDescribe) -> arenad::scenario::Scenario {
    let asked =
        json!({"disruptions": [{"id": "courier-capacity", "params": {"after": 1, "capacity": 0}}]});
    let asked = serde_json::from_value(asked).unwrap();
    arenad::scenario::expand(bench.scenario_options.as_ref(), &asked).unwrap()
}

/// The status of `item` in `area` of a recorded view.
fn status_in<'a>(view: &'a Value, area: &str, item: &str) -> &'a str {
    let areas = view["view"]["areas"].as_array().unwrap();
    let a = areas.iter().find(|a| a["id"] == area).unwrap();
    let it = a["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item);
    it.and_then(|i| i["status"].as_str()).unwrap_or("absent")
}

fn kpi_of<'a>(view: &'a Value, id: &str) -> &'a str {
    let kpis = view["view"]["kpis"].as_array().unwrap();
    kpis.iter()
        .find(|k| k["id"] == id)
        .and_then(|k| k["value"].as_str())
        .unwrap()
}

/// Milliseconds from each view's step (its last event before it) to the view.
fn view_ms(evs: &[Event]) -> Vec<i64> {
    let gap = |(prev, e): (&Event, &Event)| {
        (e.kind == EventKind::WorldView).then(|| (e.at - prev.at).num_milliseconds())
    };
    evs.iter().zip(evs.iter().skip(1)).filter_map(gap).collect()
}

/// The base incident with Tundra full after its first quote: the disruption, once the team has
/// asked for a quote, is a world event at that moment, and the twin's views follow: Tundra's lane
/// available at first and full once it fired, no order covered at first.
async fn assert_disruption_in_the_twin(hub: &Arc<Hub>, live: &Live) {
    let bench = hub.bench_describe("pharma").await.unwrap();
    let scenario = courier_full(&bench);
    let (state, _clean) = app(hub, "disrupted").await;
    let team = live.bench_team(bench.team.clone().unwrap());
    let (b, check, evs) = walk_in(&state, TASK, team, &scenario, 18_000).await;
    println!("disrupted: {:?} {:?} {check:?}", b.status, b.error);
    live::assert_recorded(&b, &evs, live);
    live_world::assert_scored(&b, &check);
    assert_views_follow(&evs);
}

/// The views: Tundra's lane available at first and full once the disruption fired, no order
/// covered at first.
fn assert_views_follow(evs: &[Event]) {
    let is_view = |e: &&Event| e.kind == EventKind::WorldView;
    let views: Vec<&Value> = evs.iter().filter(is_view).map(|e| &e.payload).collect();
    assert!(!views.is_empty());
    let tundra = "lane:Tundra Medical Courier|US";
    assert_eq!(status_in(views[0], "carriers", tundra), "available");
    assert_eq!(kpi_of(views[0], "orders_covered"), "0 of 8");
    let is_fired =
        |e: &&Event| e.kind == EventKind::WorldEvent && e.payload["kind"] == "disruption";
    match evs.iter().find(is_fired) {
        Some(fired) => {
            assert_eq!(fired.payload["data"]["id"], "courier-capacity");
            let after = evs.iter().filter(is_view).find(|v| v.seq > fired.seq);
            if let Some(v) = after {
                assert_eq!(status_in(&v.payload, "carriers", tundra), "no capacity");
            }
        }
        None => eprintln!("note: the team never asked for a quote; the disruption did not fire"),
    }
    let ms = view_ms(evs);
    let mean = ms.iter().sum::<i64>() / ms.len().max(1) as i64;
    let max = ms.iter().max();
    println!(
        "{} views; ms to each view: mean {mean}, max {max:?}",
        views.len()
    );
}

fn missing() -> Option<String> {
    let found = Command::new("sculptured")
        .arg("--version")
        .env("PATH", path_with_sculptured())
        .output();
    let uv = Command::new("uv").arg("--version").output();
    match (found.is_err(), uv.is_err()) {
        (true, _) => Some("no sculptured program".into()),
        (_, true) => Some("no uv (the simulated parts run with it)".into()),
        _ => None,
    }
}

fn build() {
    let packages = [
        "arena-pharma",
        "arena-erpnext",
        "arena-sculptured",
        "arena-twenty-crm",
    ];
    let mut args = vec!["build", "-q"];
    packages.iter().for_each(|p| args.extend(["-p", p]));
    let st = Command::new("cargo")
        .args(args)
        .current_dir(arena())
        .status()
        .unwrap();
    assert!(st.success(), "building the package binaries failed");
}

#[test]
#[ignore = "live: runs a real model on a whole world (see the module doc)"]
fn live_pharma_a_disruption_shows_in_the_twin() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    let Some(live) = Live::any("live_pharma_a_disruption_shows_in_the_twin") else {
        return;
    };
    build();
    let stack = Stack::new("pd");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(assert_disruption_in_the_twin(&hub, &live));
}

#[test]
#[ignore = "live: runs a real model on a whole world (see the module doc)"]
fn live_pharma_team_runs_the_incident_and_arena_scores_it() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    let Some(live) = Live::any("live_pharma_team_runs_the_incident_and_arena_scores_it") else {
        return;
    };
    build();
    let stack = Stack::new("pw");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(assert_team_run(&hub, &live));
}
