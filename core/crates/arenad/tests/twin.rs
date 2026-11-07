//! A run's scenario and the world's view. A scenario asked for on `POST /api/runs` is expanded
//! with the benchmark's templates into the task's setup and recorded in `RunStart`; the world's
//! view is recorded after the setup and after every step that wrote, a fork's views include those
//! before its fork point, and a view that fails records nothing and fails nothing. Without a
//! model: runs an external system starts and finishes with no agent (the setup and the views are
//! made by the world). `live_`: agents on a real model write (see `support/live.rs`); the views
//! expected follow from what was recorded.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/live.rs"]
mod live;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use arenad::{app::AppState, model::*};
use live::Live;
use packages::{package_root, toy, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

/// Runs `body` on the live model, when there is one (else the test was skipped, said why).
fn run_live<F: Future<Output = ()>>(live: Option<Live>, body: impl FnOnce(Live) -> F) {
    if let Some(live) = live {
        run(body(live));
    }
}

/// One disruption the builder may add: `jam` on part `a`, after `{after}` calls.
fn options() -> Value {
    let after =
        json!({"name": "after", "label": "After", "kind": "int", "min": 0, "max": 9, "default": 1});
    let jam = json!({"id": "jam", "label": "Jam", "description": "The box jams after {after} calls",
        "part": "a", "params": [after],
        "template": {"id": "jam-{after}", "when": {"after_calls": "{after}"}, "do": {"kind": "jam"},
                     "text": "The box jams"}});
    json!({"base": ["t1"], "disruptions": [jam], "setup": {"a": {"tool": "a_poke", "args": {}}}})
}

/// Parts `a` and `b`; the view reads `b`. `extra` goes to the benchmark's arguments.
fn world(tag: &str, extra: &[&str]) -> ToyRoot {
    world_with(tag, extra, &[])
}

/// `world`, with `part` given to `b`'s integration as well.
fn world_with(tag: &str, extra: &[&str], part: &[&str]) -> ToyRoot {
    let options = options().to_string();
    let mut args = vec![
        "bench", "--scored", "--part", "a=toy-a", "--part", "b=toy-b",
    ];
    args.extend(["--view", "b", "--scenario-options", options.as_str()]);
    args.extend(extra);
    package_root(
        tag,
        &[
            toy("toy-a", "integration", &["part", "--prefix", "a_"]),
            toy(
                "toy-b",
                "integration",
                &[&["part", "--prefix", "b_"], part].concat(),
            ),
            toy("toybench", "benchmark", &args),
        ],
    )
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    _hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

async fn setup(tag: &str, root: ToyRoot) -> Fx {
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("twin", tag).await;
    let (state, base) =
        app_support::app_for(store, hub.hub.clone(), Some("toybench"), |_| {}).await;
    Fx {
        state,
        base,
        _hub: hub,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let url = format!("{}{path}", self.base);
        app_support::send(reqwest::Client::new().post(url).json(&body)).await
    }

    async fn get(&self, path: &str) -> Value {
        let url = format!("{}{path}", self.base);
        let (status, v) = app_support::send(reqwest::Client::new().get(url)).await;
        assert_eq!(status, 200, "{v}");
        v
    }

    /// Starts a run of `t1` in `scenario` that an external system drives, with no agent, and
    /// finishes it: its branch and events.
    async fn external_run(&self, scenario: Value) -> (String, Vec<Event>) {
        let team = json!({"id": "own", "name": "Own", "agents": []});
        let body =
            json!({"task_id": "t1", "team": team, "runner": "external", "scenario": scenario});
        let (status, v) = self.post("/api/runs", body).await;
        assert_eq!(status, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let (status, v) = self
            .post(&format!("/api/branches/{branch}/finish"), json!({}))
            .await;
        assert_eq!(status, 200, "{v}");
        let b = self.state.store.branch(&branch).await.unwrap();
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        (
            branch.clone(),
            self.state.store.events(&branch).await.unwrap(),
        )
    }

    /// Starts a run of `t1` with `team` in `scenario`; its branch, run to its end, and its events.
    async fn run_in(&self, live: &Live, team: &Team, scenario: Value) -> (String, Vec<Event>) {
        let body = json!({"task_id": "t1", "team": team, "scenario": scenario});
        let (status, v) = self.post("/api/runs", body).await;
        assert_eq!(status, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let (b, evs) = live::ended_ok(&self.state, &branch).await;
        live::assert_recorded(&b, &evs, live);
        (branch, evs)
    }

    /// Forks `parent` at `at_seq` with no change; the new branch's id and events, run to its end.
    async fn fork_to_end(&self, parent: &str, at_seq: i64) -> (String, Vec<Event>) {
        let fork = json!({"at_seq": at_seq});
        let (status, v) = self
            .post(&format!("/api/branches/{parent}/fork"), fork)
            .await;
        assert_eq!(status, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        let (_b, evs) = live::ended_ok(&self.state, &child).await;
        (child, evs)
    }
}

fn steps<'a>(evs: &'a [Event], actor: &str) -> Vec<&'a Event> {
    live::calls_by(evs, actor)
}

/// `(seq, n)` of each view: the step it follows and the item's status.
fn drawn(views: &Value) -> Vec<(i64, String)> {
    let one = |v: &Value| {
        let status = v["view"]["areas"][0]["items"][0]["status"]
            .as_str()
            .unwrap();
        (v["seq"].as_i64().unwrap(), status.to_string())
    };
    views.as_array().unwrap().iter().map(one).collect()
}

/// The views the record calls for: one after the setup's last step (or after `RunStart` when it
/// wrote nothing), then one after every later step that wrote, each showing how many times `b`
/// had been poked by then.
fn views_wanted(evs: &[Event]) -> Vec<(i64, String)> {
    let first = steps(evs, "setup").last().map_or(1, |e| e.seq);
    let mut out = vec![(first, "0".to_string())];
    let mut pokes = 0;
    for e in evs
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall && e.seq > first)
    {
        let wrote = e.payload["writes"]
            .as_array()
            .is_some_and(|w| !w.is_empty());
        if e.payload["name"] == "b_poke" && e.payload["error"].is_null() {
            pokes += 1;
        }
        if wrote {
            out.push((e.seq, pokes.to_string()));
        }
    }
    out
}

/// The chosen disruption is expanded into `a`'s setup call and recorded in `RunStart`, a bad
/// choice is refused, and the benchmark's options are offered.
#[test]
fn a_scenario_is_expanded_into_the_setup_and_recorded() {
    run(async {
        let fx = setup("scen", world("scen", &[])).await;
        let offered = fx.get("/api/benchmark").await;
        assert_eq!(offered["scenario_options"]["disruptions"][0]["id"], "jam");
        assert_eq!(offered["view"], true);
        let bad = json!({"disruptions": [{"id": "jam", "params": {"after": 99}}]});
        let team = json!({"id": "own", "name": "Own", "agents": []});
        let body = json!({"task_id": "t1", "team": team, "runner": "external", "scenario": bad});
        let (status, v) = fx.post("/api/runs", body).await;
        assert_eq!(
            (
                status,
                v["error"].as_str().unwrap().contains("after must be from")
            ),
            (400, true),
            "{v}"
        );
        let (_, evs) = fx
            .external_run(json!({"disruptions": [{"id": "jam", "params": {"after": 2}}]}))
            .await;
        let start = &evs[0].payload["scenario"]["disruptions"][0];
        assert_eq!((&start["id"], &start["part"]), (&json!("jam"), &json!("a")));
        assert_eq!(start["disruption"]["when"]["after_calls"], 2, "{start}");
        let set = steps(&evs, "setup");
        assert_eq!(set.len(), 1, "a setup call added for the part");
        assert_eq!(set[0].payload["name"], "a_poke");
        assert_eq!(set[0].payload["args"]["disruptions"][0]["id"], "jam-2");
        let (_, plain) = fx.external_run(json!({"disruptions": []})).await;
        assert!(plain[0].payload.get("scenario").is_none() && steps(&plain, "setup").is_empty());
    })
}

/// A view after the setup and after each step that wrote (not after a read), and a fork has the
/// views before its fork point, then its own.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_views_follow_the_writing_steps_and_forks_inherit_them() {
    run_live(
        Live::any("live_views_follow_the_writing_steps_and_forks_inherit_them"),
        views_follow_the_writing_steps_and_forks_inherit_them,
    );
}

async fn views_follow_the_writing_steps_and_forks_inherit_them(live: Live) {
    let fx = setup("views", world("views", &[])).await;
    let poke = "Call b_poke once, then b_peek once, then b_poke once more, then stop.";
    let team = live.team(vec![live.agent("first", poke), live.agent("second", poke)]);
    let (parent, evs) = fx
        .run_in(
            &live,
            &team,
            json!({"disruptions": [{"id": "jam", "params": {}}]}),
        )
        .await;
    let views = fx.get(&format!("/api/branches/{parent}/views")).await;
    let want = views_wanted(&evs);
    assert_eq!(drawn(&views), want);
    let set = steps(&evs, "setup");
    assert!(views[0]["event_seq"].as_i64().unwrap() > set[0].seq);
    let at = live::first_of(&evs, EventKind::AgentStart, "second").unwrap();
    let (child, cevs) = fx.fork_to_end(&parent, at).await;
    assert_eq!(
        cevs[0].payload["scenario"]["disruptions"][0]["id"], "jam",
        "the fork keeps it"
    );
    let inherited = drawn(&fx.get(&format!("/api/branches/{child}/views")).await);
    let before: Vec<_> = want.iter().filter(|(s, _)| *s < at).cloned().collect();
    assert_eq!(inherited[..before.len()], before[..]);
    assert_eq!(inherited, views_wanted(&cevs));
}

/// A run without a scenario still gets a view after its start; a view that fails records nothing
/// and the run goes on.
#[test]
fn a_failing_view_records_nothing_and_fails_nothing() {
    run(async {
        let fx = setup("noview", world("noview", &["--view-fails"])).await;
        let (_, evs) = fx.external_run(Value::Null).await;
        assert!(evs.iter().all(|e| e.kind != EventKind::WorldView));
        assert_eq!(evs.last().unwrap().kind, EventKind::Check);
        let fx = setup("startview", world("startview", &[])).await;
        let (branch, _) = fx.external_run(Value::Null).await;
        let views = fx.get(&format!("/api/branches/{branch}/views")).await;
        assert_eq!(drawn(&views), [(1, "0".to_string())], "after RunStart");
    })
}

/// A read that changes only its part's internal keys (`_peeks/n`) is saved, so a fork restores
/// them exactly, but it is recorded as no write and gets no view.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_internal_keys_are_saved_but_make_no_write_and_no_view() {
    run_live(
        Live::any("live_internal_keys_are_saved_but_make_no_write_and_no_view"),
        internal_keys_are_saved_but_make_no_write_and_no_view,
    );
}

/// Each peek changed only the internal key: no write, but a save.
fn assert_internal_only(peeks: &[&Event]) {
    for peek in peeks {
        let p = &peek.payload;
        let keys = (&p["writes"], &p["internal"]);
        assert_eq!(keys, (&json!([]), &json!(["_peeks/n"])), "{p}");
        assert!(
            p["saves"]["b"].is_string(),
            "saved for its internal key: {p}"
        );
    }
}

async fn internal_keys_are_saved_but_make_no_write_and_no_view(live: Live) {
    let root = world_with("internal", &[], &["--internal-peeks"]);
    let fx = setup("internal", root).await;
    let team = live.team(vec![
        live.agent("first", "Call b_peek twice, then b_poke once, then stop."),
        live.agent("second", "Call b_peek once, then stop."),
    ]);
    let (parent, evs) = fx.run_in(&live, &team, Value::Null).await;
    let peeks = live::calls_named(&evs, "b_peek");
    assert_internal_only(&peeks);
    let views = fx.get(&format!("/api/branches/{parent}/views")).await;
    let why = "after RunStart and the writes, not the peeks";
    assert_eq!(drawn(&views), views_wanted(&evs), "{why}");
    let at = live::first_of(&evs, EventKind::AgentStart, "second").unwrap();
    let before = peeks.iter().filter(|e| e.seq < at).count() as i64;
    let (_child, cevs) = fx.fork_to_end(&parent, at).await;
    let again = live::calls_named(&cevs, "b_peek")
        .into_iter()
        .find(|e| e.seq >= at);
    let Some(again) = again else {
        eprintln!("note: the child's agent did not peek this time");
        return;
    };
    let result = &again.payload["result"];
    let why = "the fork restored the peeks counted";
    assert_eq!(result["peeks"], before + 1, "{why}: {result}");
}
