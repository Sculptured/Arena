//! Fork: go back to a step of a finished branch, maybe change an agent, and run again from there
//! with the world as it was. Without a model: what is refused, and a run recorded before
//! integrations (written to the store by hand as data, as such runs were stored) still shows
//! its rows, compares and deletes. `live_`: forks of runs on a real model (see
//! `support/live.rs`); where a fork starts for each kind of agent is planned on recorded events
//! in `src/fork.rs`'s own tests.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/legacy.rs"]
mod legacy;
#[path = "support/live.rs"]
mod live;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock},
};

use arenad::{
    app::AppState,
    fork::{self, ForkError},
    model::*,
    runner,
};
use legacy::{legacy_run, LegacyDbs};
use live::Live;
use packages::{plain_hub, TestHub};
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

/// A team for data only (never run).
fn team() -> Team {
    let agent = |id: &str| AgentSpec {
        tools: Vec::new(),
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("m".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
        lead: false,
        url: None,
        headers: Default::default(),
    };
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

struct Fx {
    state: Arc<AppState>,
    hub: TestHub,
    _clean: Cleanup,
}

/// An app with the real drivers on the configured providers.
async fn setup(tag: &str) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("fork", tag).await;
    let (state, _) = app_support::app_for(store, hub.hub.clone(), Some("retail"), |_| {}).await;
    Fx {
        state,
        hub,
        _clean: clean,
    }
}

const ORDER: &str = "#W7181492";

/// The retail benchmark's own team on `live`.
async fn bench_team(fx: &Fx, live: &Live) -> Team {
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    live.bench_team(bench.team.unwrap())
}

/// Runs `team` on task 27 to its end: the branch id, the branch and its events.
async fn parent(fx: &Fx, team: Team) -> (String, Branch, Vec<Event>) {
    let (_run, id) = runner::start_run(&fx.state, "27", team).await.unwrap();
    let (b, evs) = live::ended_ok(&fx.state, &id).await;
    (id, b, evs)
}

/// The events before the fork point at `at`, as comparable tuples: a child's timeline starts
/// with its parent's.
fn events_before(evs: &[Event], at: i64) -> Vec<(i64, EventKind, Value)> {
    evs.iter()
        .filter(|e| e.seq < at)
        .map(|e| (e.seq, e.kind, e.payload.clone()))
        .collect()
}

/// An exact fork of a world of exact saves is restored from the save; the run's first branch
/// was restored from nothing. `c` is a fork of `b` (id `parent`) at `at`, in the same run.
fn assert_child_of(c: &Branch, b: &Branch, parent: &str, at: i64) {
    assert_eq!(c.parent_id.as_deref(), Some(parent));
    assert_eq!(c.start_seq, Some(at));
    assert_eq!(c.run_id, b.run_id);
    assert_eq!((b.restore_mode, b.restored.len()), (None, 0));
    assert_eq!(c.restore_mode, Some(parts::proto::Mode::Exact));
    let retail = RestoredPart {
        part: "retail".into(),
        how: RestoredHow::Save,
    };
    assert_eq!(c.restored, [retail]);
}

/// The order as the parent's world had it just before `at`: in its last save before then, else
/// the base save.
async fn order_before(fx: &Fx, evs: &[Event], at: i64) -> Value {
    let save = evs
        .iter()
        .rev()
        .filter(|e| e.seq < at)
        .find_map(|e| e.payload["saves"]["retail"].as_str())
        .unwrap_or("base_retail_0_1_0_retail");
    let key = format!("orders/{ORDER}");
    fx.hub.inspect_save(save, &[&key]).await[&key]["doc"].clone()
}

/// The child's first read of the order, if it read it before writing anything.
fn first_read_before_writes(cevs: &[Event], from: i64) -> Option<Value> {
    for e in cevs
        .iter()
        .filter(|e| e.seq >= from && e.kind == EventKind::ToolCall)
    {
        if e.payload["writes"]
            .as_array()
            .is_some_and(|w| !w.is_empty())
        {
            return None;
        }
        if e.payload["name"] == "get_order_details" && e.payload["error"].is_null() {
            return Some(e.payload["result"].clone());
        }
    }
    None
}

/// A fork at the resolver's start: it starts there, in the same run, its timeline begins with
/// its parent's, its world is restored exactly from the save as it was at that point (what the
/// resolver reads first), the triage is not run again, and a bad step or agent makes no branch.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_fork_restores_the_world_at_its_fork_point() {
    run_live(
        Live::any("live_a_fork_restores_the_world_at_its_fork_point"),
        a_fork_restores_the_world_at_its_fork_point,
    );
}

/// A fork at a step the branch doesn't have, or changing an agent the team doesn't have, is
/// refused and makes no branch.
async fn assert_bad_forks_make_nothing(fx: &Fx, pid: &str, at: i64) {
    let e = fork::fork(&fx.state, pid, 999, HashMap::new())
        .await
        .unwrap_err();
    assert!(matches!(e, ForkError::UnknownSeq), "{e:?}");
    let change = Change {
        model: Some("x/y".into()),
        ..Default::default()
    };
    let ghost = HashMap::from([("ghost".to_string(), change)]);
    let e = fork::fork(&fx.state, pid, at, ghost).await.unwrap_err();
    assert!(
        matches!(e, ForkError::UnknownAgent(ref a) if a == "ghost"),
        "{e:?}"
    );
    assert_eq!(fx.state.store.runs().await.unwrap()[0].branches.len(), 1);
}

async fn a_fork_restores_the_world_at_its_fork_point(live: Live) {
    let fx = setup("world").await;
    let (pid, b, pevs) = parent(&fx, bench_team(&fx, &live).await).await;
    let at = live::first_of(&pevs, EventKind::AgentStart, "resolver").unwrap();
    assert_bad_forks_make_nothing(&fx, &pid, at).await;
    let resp = fork::fork(&fx.state, &pid, at, HashMap::new())
        .await
        .unwrap();
    assert_eq!((resp.start_seq, resp.moved_from), (at, None));
    let (c, cevs) = live::ended_ok(&fx.state, &resp.branch_id).await;
    assert_child_of(&c, &b, &pid, at);
    live::assert_recorded(&c, &cevs, &live);
    assert_eq!(events_before(&cevs, at), events_before(&pevs, at));
    assert!(cevs
        .iter()
        .filter(|e| e.seq >= at)
        .all(|e| e.agent.as_deref() != Some("triage")));
    let want = order_before(&fx, &pevs, at).await;
    match first_read_before_writes(&cevs, at) {
        Some(read) => assert_eq!(read["status"], want["status"], "{read}"),
        None => eprintln!("note: the resolver did not read the order first this time"),
    }
}

/// A prompt change for the resolver: the child stores its effective team and the change as
/// given, and (with arenad's own loop) every model call of the resolver in it carries the new
/// system prompt.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_fork_applies_a_prompt_change() {
    run_live(
        Live::any("live_a_fork_applies_a_prompt_change"),
        a_fork_applies_a_prompt_change,
    );
}

async fn a_fork_applies_a_prompt_change(live: Live) {
    let fx = setup("prompt").await;
    let team = bench_team(&fx, &live).await;
    let triage_prompt = team.agents[0].prompt.clone();
    let (pid, _b, pevs) = parent(&fx, team).await;
    let at = live::first_of(&pevs, EventKind::AgentStart, "resolver").unwrap();
    let new_prompt =
        "You are a careful resolver. Never change addresses. Read the order, then stop.";
    let change = Change {
        prompt: Some(new_prompt.into()),
        ..Default::default()
    };
    let changes = HashMap::from([("resolver".to_string(), change)]);
    let resp = fork::fork(&fx.state, &pid, at, changes).await.unwrap();
    assert_eq!(resp.start_seq, at);
    let (c, cevs) = live::ended_ok(&fx.state, &resp.branch_id).await;
    live::assert_recorded(&c, &cevs, &live);
    assert_eq!(c.team.agents[1].prompt, new_prompt);
    assert_eq!(c.team.agents[0].prompt, triage_prompt);
    assert_eq!(c.changes["resolver"]["prompt"], new_prompt);
    assert_eq!(c.fork_note, None);
    for e in cevs
        .iter()
        .filter(|e| e.seq >= at && e.kind == EventKind::ModelCall)
    {
        assert_eq!(e.payload["request"]["messages"][0]["content"], new_prompt);
    }
}

/// arenad's own agent resumes a fork at one of its model calls: the child starts there and its
/// first call sends the conversation recorded up to it.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_an_open_agent_resumes_a_fork_mid_conversation() {
    run_live(
        Live::api("live_an_open_agent_resumes_a_fork_mid_conversation"),
        an_open_agent_resumes_a_fork_mid_conversation,
    );
}

async fn an_open_agent_resumes_a_fork_mid_conversation(live: Live) {
    let fx = setup("resume").await;
    let (pid, _b, pevs) = parent(&fx, bench_team(&fx, &live).await).await;
    let calls: Vec<&Event> = pevs
        .iter()
        .filter(|e| e.kind == EventKind::ModelCall && e.agent.as_deref() == Some("resolver"))
        .collect();
    let at_call = calls.last().expect("the resolver called its model");
    let at = at_call.seq;
    let resp = fork::fork(&fx.state, &pid, at, HashMap::new())
        .await
        .unwrap();
    assert_eq!(
        (resp.start_seq, resp.moved_from, resp.reason),
        (at, None, None)
    );
    let (_c, cevs) = live::ended_ok(&fx.state, &resp.branch_id).await;
    let first = cevs
        .iter()
        .find(|e| e.seq >= at && e.kind == EventKind::ModelCall)
        .unwrap();
    assert_eq!(
        first.payload["request"]["messages"],
        at_call.payload["request"]["messages"]
    );
}

/// A branch of task 27 that has started and not ended.
async fn running_branch(fx: &Fx) -> String {
    let t = team();
    let run_id = fx.state.store.create_run("27", &t).await.unwrap();
    let branch = fx
        .state
        .store
        .create_branch(&run_id, None, &t, &json!([]), None)
        .await
        .unwrap();
    fx.state
        .store
        .append(&branch, None, EventKind::RunStart, json!({"task_id": "27"}))
        .await
        .unwrap();
    assert_eq!(
        fx.state.store.branch(&branch).await.unwrap().status,
        BranchStatus::Running
    );
    branch
}

#[test]
fn fork_while_running_is_409() {
    run(async {
        let fx = setup("running").await;
        let branch = running_branch(&fx).await;

        let e = fork::fork(&fx.state, &branch, 1, HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(e, ForkError::BranchRunning), "{e:?}");
        assert_eq!(e.to_string(), "the branch is still running");
        // Checked before the plan: even an unknown seq reports the running branch.
        let e = fork::fork(&fx.state, &branch, 999, HashMap::new())
            .await
            .unwrap_err();
        assert!(matches!(e, ForkError::BranchRunning), "{e:?}");
        // No branch was created.
        let runs = fx.state.store.runs().await.unwrap();
        assert_eq!(runs[0].branches.len(), 1);
        drop(fx.state);
    })
}

#[test]
fn fork_of_unknown_branch_is_unknown_branch() {
    run(async {
        let fx = setup("nobranch").await;
        let e = fork::fork(&fx.state, "nosuchbranch", 1, HashMap::new())
            .await
            .unwrap_err();
        assert!(
            matches!(e, ForkError::UnknownBranch(ref b) if b == "nosuchbranch"),
            "{e:?}"
        );
        assert!(e.to_string().contains("nosuchbranch"));
        assert!(fx
            .state
            .store
            .find_branch("nosuchbranch")
            .await
            .unwrap()
            .is_none());
        drop(fx.state);
    })
}

/// A copy of the retail base save under the legacy name `snap_<tag>`, with order #W7181492's
/// exchange requested (the snapshot of an old run's write). The base is made in a namespace of
/// the test's own; the copy is dropped by `dbs`.
async fn legacy_snapshot(dbs: &LegacyDbs, tag: &str) -> (String, String) {
    let seeded = TestHub::new(&[]);
    let bench = seeded.hub.bench_describe("retail").await.unwrap();
    let base = arenad::world::ensure_base(&seeded.hub, &bench)
        .await
        .unwrap();
    let base_copy = format!("branch_{tag}base");
    dbs.copy(&base_copy, &format!("ps_{}{}", seeded.ns, base["retail"]))
        .await;
    let snap = format!("snap_{tag}");
    dbs.copy(&snap, &base_copy).await;
    let sql = format!(
        "UPDATE orders SET doc = jsonb_set(doc, '{{status}}', '\"exchange requested\"') WHERE id = '{ORDER}'"
    );
    dbs.execute(&snap, &sql).await;
    (snap, base_copy)
}

/// An app on a hub without a namespace (legacy names work), with two runs recorded before
/// integrations: `a` ends with the snapshot's world, `b` with the base's.
/// The snapshot, run `a`, and branches `a` and `b` of a `LegacyFx`.
struct Names {
    snap: String,
    run_a: String,
    a: String,
    b: String,
}

struct LegacyFx {
    state: Arc<AppState>,
    base: String,
    http: reqwest::Client,
    names: Names,
    /// Seq of the write whose snapshot is `snap`.
    write: i64,
    _clean: Cleanup,
    dbs: LegacyDbs,
}

impl LegacyFx {
    /// Two old runs of the one-agent `solo` team.
    async fn new(solo: &Team) -> LegacyFx {
        let id = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let tag = format!("t8{}_{id}", std::process::id());
        let dbs = LegacyDbs::new(&tag);
        let (snap, base_copy) = legacy_snapshot(&dbs, &tag).await;
        let (store, clean) = app_support::meta_store("fork", "legacy").await;
        let hub = plain_hub(&dbs.saves);
        let (state, base) = app_support::app_for(store, hub, Some("retail"), |_| {}).await;
        let (run_a, a) = legacy_run(&state.store, &dbs, solo, &snap, &snap).await;
        let (_, b) = legacy_run(&state.store, &dbs, solo, &snap, &base_copy).await;
        let evs = state.store.events(&a).await.unwrap();
        let write = evs
            .iter()
            .find(|e| e.kind == EventKind::ToolCall)
            .unwrap()
            .seq;
        LegacyFx {
            http: reqwest::Client::new(),
            names: Names { snap, run_a, a, b },
            write,
            state,
            base,
            _clean: clean,
            dbs,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(self.http.get(self.url(path))).await
    }

    /// Rows at the write read its snapshot.
    async fn check_rows(&self) {
        let path = format!(
            "/api/branches/{}/rows?seq={}&keys=orders/%23W7181492",
            self.names.a, self.write
        );
        let (st, v) = self.get(&path).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(
            v["rows"][format!("orders/{ORDER}")]["doc"]["status"],
            "exchange requested",
            "{v}"
        );
    }

    /// Compare diffs the branch databases.
    async fn check_compare(&self) {
        let (st, v) = self
            .get(&format!(
                "/api/compare?a={}&b={}",
                self.names.a, self.names.b
            ))
            .await;
        assert_eq!(st, 200, "{v}");
        let rows = v["differing_rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{v}");
        assert_eq!(rows[0]["key"], format!("orders/{ORDER}"));
        assert_eq!(rows[0]["a"]["doc"]["status"], "exchange requested");
        assert_eq!(rows[0]["b"]["doc"]["status"], "delivered");
    }

    /// A fork from the model call after the write starts from the snapshot: its first read sees
    /// the write.
    async fn check_fork(&self) {
        let at = self.write + 1;
        let req = self
            .http
            .post(self.url(&format!("/api/branches/{}/fork", self.names.a)));
        let (st, v) = app_support::send(req.json(&json!({"at_seq": at}))).await;
        assert_eq!((st, v["start_seq"].as_i64()), (200, Some(at)), "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        // Its instance, with no namespace: dropped when the branch ends, and at the end if not.
        let instance = arenad::world::instance_id(&child, "retail");
        self.dbs.track(&format!("pi_{instance}"));
        let ended = live::ended(&self.state, &child).await;
        let cevs = self.state.store.events(&child).await.unwrap();
        match first_read_before_writes(&cevs, at) {
            Some(read) => assert_eq!(
                read["status"], "exchange requested",
                "{read} {:?}",
                ended.error
            ),
            None => eprintln!("note: the agent did not read the order first this time"),
        }
    }

    /// Deleting run `a` drops its snapshot and branch database, not `b`'s.
    async fn check_delete(&self) {
        let req = self
            .http
            .delete(self.url(&format!("/api/runs/{}", self.names.run_a)));
        let (st, v) = app_support::send(req).await;
        // The snapshot and a's database; the child's `branch_<id>` never existed, so isn't counted.
        assert_eq!((st, v["dropped"].as_u64()), (200, Some(2)), "{v}");
        let admin = pg::admin();
        let exists = async |db: &str| admin.exists(db).await.unwrap();
        assert!(!exists(&self.names.snap).await, "the snapshot is dropped");
        assert!(
            !exists(&format!("branch_{}", self.names.a)).await,
            "a's database is dropped"
        );
        assert!(
            exists(&format!("branch_{}", self.names.b)).await,
            "b keeps its own"
        );
    }
}

/// A run recorded before integrations (a `snapshot` per write, no saves) still shows rows and
/// compares (through its `branch_<id>` database), and deleting it drops its snapshot and branch
/// database.
#[test]
fn old_run_still_shows_rows_compares_and_deletes() {
    run(async {
        let fx = LegacyFx::new(&team()).await;
        fx.check_rows().await;
        fx.check_compare().await;
        fx.check_delete().await;
    })
}

/// An old run (its agents were all arenad's own) still forks from its snapshot: a fork at the
/// model call after the write starts from the snapshot, so the world has the write.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_old_run_still_forks_from_its_snapshot() {
    run_live(
        Live::api("live_old_run_still_forks_from_its_snapshot"),
        old_run_still_forks_from_its_snapshot,
    );
}

async fn old_run_still_forks_from_its_snapshot(live: Live) {
    let prompt = "You are a retail support agent. First read order #W7181492 with get_order_details, then stop.";
    let fx = LegacyFx::new(&live.team(vec![live.agent("resolver", prompt)])).await;
    fx.check_fork().await;
    fx.check_delete().await;
}
