#[path = "support/app.rs"]
mod app_support;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use arenad::{
    app::{AppState, Config},
    exec::BranchCtx,
    model::*,
    store::Store,
    world::World,
};
use packages::TestHub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: None,
        prompt: "p".into(),
        provider: None,
    }
}

struct Fx {
    store: Arc<Store>,
    hub: TestHub,
    world: Arc<World>,
    _clean: Cleanup,
    branch: String,
    url: String,
}

async fn setup(tag: &str) -> Fx {
    setup_with(tag, &[], "retail").await
}

/// A run of a two-agent team with one branch, recorded in `store`.
async fn new_branch(store: &Store) -> (Team, String) {
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    };
    let run_id = store.create_run("task", &team).await.unwrap();
    let branch = store
        .create_branch(&run_id, None, &team, &json!([]), None)
        .await
        .unwrap();
    (team, branch)
}

async fn setup_with(tag: &str, extra: &[PathBuf], bench: &str) -> Fx {
    let hub = TestHub::new(extra);
    let (store, clean) = app_support::meta_store("mcp", tag).await;
    let (team, branch) = new_branch(&store).await;
    let world = hub.start_world(bench, &branch).await;
    let state = AppState::new(store.clone(), hub.hub.clone(), Config::new());
    let state = Arc::new(state.with_benchmark(bench));
    let ctx = BranchCtx::new(branch.clone(), world.clone(), team, "task".into());
    state
        .live
        .write()
        .unwrap()
        .insert(branch.clone(), Arc::new(ctx));
    let (listener, base) = app_support::bind().await;
    app_support::serve(listener, &state);
    Fx {
        store,
        hub,
        world,
        _clean: clean,
        branch,
        url: format!("{base}/mcp"),
    }
}

async fn teardown(fx: Fx) {
    fx.world.stop().await;
}

async fn rpc(fx: &Fx, agent: &str, body: Value) -> (u16, Value) {
    let r = reqwest::Client::new()
        .post(format!("{}/{}/{}", fx.url, fx.branch, agent))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let text = r.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn call(fx: &Fx, agent: &str, name: &str, args: Value) -> Value {
    let (s, v) = rpc(
        fx,
        agent,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}}),
    )
    .await;
    assert_eq!(s, 200);
    v["result"].clone()
}

fn address_args() -> Value {
    json!({"user_id": "ethan_garcia_1261", "address1": "7 Mcp Road",
        "address2": "", "city": "New York", "state": "NY", "country": "USA", "zip": "10001"})
}

fn names(v: &Value) -> Vec<String> {
    v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn base_save(fx: &Fx) -> String {
    fx.world.saves()["retail"].clone()
}

fn instance(fx: &Fx) -> Value {
    json!({"instance": fx.world.instance("retail").unwrap()})
}

#[test]
fn initialize_and_notification() {
    run(async {
        let fx = setup("init").await;
        let (s, v) = rpc(
            &fx,
            "triage",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(v["result"]["serverInfo"]["name"], "arena-retail");
        assert_eq!(v["result"]["capabilities"], json!({"tools":{}}));
        let (s, _) = rpc(
            &fx,
            "triage",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert_eq!(s, 202);
        let (s, _) = rpc(
            &fx,
            "nobody",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
        assert_eq!(s, 404);
        teardown(fx).await;
    })
}

#[test]
fn tools_list_has_handoff_only_for_first_agent() {
    run(async {
        let fx = setup("list").await;
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let (_, a) = rpc(&fx, "triage", req.clone()).await;
        let (_, b) = rpc(&fx, "resolver", req).await;
        assert!(names(&a).contains(&"handoff".to_string()));
        assert!(names(&a).contains(&"get_user_details".to_string()));
        assert!(!names(&b).contains(&"handoff".to_string()));
        assert!(names(&b).contains(&"modify_user_address".to_string()));
        // MCP shape: name, description, inputSchema.
        let t = &b["result"]["tools"][0];
        assert!(
            t["name"].is_string() && t["description"].is_string() && t["inputSchema"].is_object(),
            "{t}"
        );
        teardown(fx).await;
    })
}

#[test]
fn triage_write_is_refused_and_recorded() {
    run(async {
        let fx = setup("refuse").await;
        let r = call(&fx, "triage", "modify_user_address", address_args()).await;
        assert_eq!(r["isError"], true);
        assert_eq!(
            r["content"][0]["text"],
            "Write tools are not available to this agent"
        );
        let evs = fx.store.events(&fx.branch).await.unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, EventKind::ToolCall);
        assert_eq!(evs[0].agent.as_deref(), Some("triage"));
        let p: ToolCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        assert!(p.error.is_some());
        assert!(p.snapshot.is_none() && p.saves.is_none());
        // The world is untouched.
        assert!(fx
            .hub
            .diff(instance(&fx), json!({"save": base_save(&fx)}))
            .await
            .is_empty());
        teardown(fx).await;
    })
}

#[test]
fn run_records_saves_per_part() {
    run(async {
        let fx = setup("saves").await;
        let r = call(&fx, "resolver", "modify_user_address", address_args()).await;
        assert_eq!(r["isError"], false, "{r}");
        assert!(r["structuredContent"].is_object());
        let p = payload(&fx, 0).await;
        assert_eq!(p.part.as_deref(), Some("retail"));
        let save = p.saves.as_ref().expect("saves")["retail"].clone();
        assert!(save.starts_with("s_") && save.len() == 34, "{save}");
        // `snapshot` still names the retail save, for the old UIs.
        assert_eq!(p.snapshot.as_deref(), Some(save.as_str()));
        assert!(p.clock_ms.is_some_and(|c| c >= 0));
        // The save is the world just after the write.
        assert!(fx
            .hub
            .diff(instance(&fx), json!({"save": save}))
            .await
            .is_empty());
        let key = "users/ethan_garcia_1261";
        let user = fx.hub.inspect_save(&save, &[key]).await;
        assert_eq!(user[key]["doc"]["address"]["address1"], "7 Mcp Road");
        // A read names its part and saves nothing.
        let read = json!({"user_id": "ethan_garcia_1261"});
        assert_eq!(
            call(&fx, "resolver", "get_user_details", read).await["isError"],
            false
        );
        let p = payload(&fx, 1).await;
        assert_eq!(p.part.as_deref(), Some("retail"));
        assert!(p.saves.is_none() && p.snapshot.is_none());
        teardown(fx).await;
    })
}

/// The `n`-th recorded event, as a tool call.
async fn payload(fx: &Fx, n: usize) -> ToolCallPayload {
    let evs = fx.store.events(&fx.branch).await.unwrap();
    serde_json::from_value(evs[n].payload.clone()).unwrap()
}

#[test]
fn bad_calls_are_tool_errors_not_500() {
    run(async {
        let fx = setup("bad").await;
        let cases = [
            json!({"name":"nope","arguments":{}}),
            json!({"name":"get_user_details","arguments":"x"}),
            json!({"name":"get_user_details","arguments":{}}),
            json!({"name":"get_user_details"}),
            json!({"arguments":{}}),
        ];
        for (i, params) in cases.iter().enumerate() {
            let (s, v) = rpc(
                &fx,
                "resolver",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}),
            )
            .await;
            assert_eq!(s, 200, "case {i}");
            assert_eq!(v["result"]["isError"], true, "case {i}: {v}");
            let evs = fx.store.events(&fx.branch).await.unwrap();
            assert_eq!(evs.len(), i + 1, "case {i}");
        }
        teardown(fx).await;
    })
}

#[test]
fn handoff_records_once_and_is_not_for_last_agent() {
    run(async {
        let fx = setup("handoff").await;
        let r = call(&fx, "triage", "handoff", json!({})).await;
        assert_eq!(r["isError"], true);
        let r = call(&fx, "triage", "handoff", json!({"note":"  "})).await;
        assert_eq!(r["isError"], true);
        let r = call(&fx, "triage", "handoff", json!({"note":"user wants X"})).await;
        assert_eq!(r["isError"], false);
        let r = call(&fx, "triage", "handoff", json!({"note":"again"})).await;
        assert_eq!(r["isError"], true);
        let r = call(&fx, "resolver", "handoff", json!({"note":"n"})).await;
        assert_eq!(r["isError"], true);
        let evs = fx.store.events(&fx.branch).await.unwrap();
        let kinds: Vec<_> = evs.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::ToolCall,
                EventKind::ToolCall,
                EventKind::Handoff,
                EventKind::ToolCall,
                EventKind::ToolCall
            ]
        );
        let h: HandoffPayload = serde_json::from_value(evs[2].payload.clone()).unwrap();
        assert_eq!(h.note, "user wants X");
        assert_eq!(evs[2].agent.as_deref(), Some("triage"));
        teardown(fx).await;
    })
}

fn addr_for(user: &str, line: &str) -> Value {
    json!({"user_id": user, "address1": line, "address2": "", "city": "New York",
           "state": "NY", "country": "USA", "zip": "10001"})
}

#[test]
fn parallel_writes_snapshot_in_order() {
    run(async {
        let fx = setup("par").await;
        let (u1, u2) = ("ethan_garcia_1261", "isabella_johansson_2152");
        let (a, b) = tokio::join!(
            call(
                &fx,
                "resolver",
                "modify_user_address",
                addr_for(u1, "1 Alpha Way")
            ),
            call(
                &fx,
                "resolver",
                "modify_user_address",
                addr_for(u2, "2 Beta Way")
            ),
        );
        assert_eq!(a["isError"], false, "{a}");
        assert_eq!(b["isError"], false, "{b}");
        let evs = fx.store.events(&fx.branch).await.unwrap();
        assert_eq!(evs.len(), 2);
        assert!(evs[0].seq < evs[1].seq);
        let mut seen = vec![];
        for (i, e) in evs.iter().enumerate() {
            let p: ToolCallPayload = serde_json::from_value(e.payload.clone()).unwrap();
            let line = p.args["address1"].as_str().unwrap().to_string();
            let save = p.saves.expect("saves")["retail"].clone();
            let docs = fx
                .hub
                .inspect_save(&save, &[&format!("users/{u1}"), &format!("users/{u2}")])
                .await
                .to_string();
            seen.push(line.clone());
            for l in ["1 Alpha Way", "2 Beta Way"] {
                let expected = seen.iter().any(|s| s == l);
                assert_eq!(docs.contains(l), expected, "event {i} save, line {l}");
            }
        }
        teardown(fx).await;
    })
}

#[test]
fn failed_save_is_recorded() {
    run(async {
        // The toy's first save (the base) works; later ones fail.
        let root = packages::toy_world("nosave", &["--save-fails-after", "1"], &[]);
        let fx = setup_with("savefail", std::slice::from_ref(&root.0), "toybench").await;
        let r = call(&fx, "resolver", "poke", json!({})).await;
        assert_eq!(r["isError"], false, "{r}");
        let p = payload(&fx, 0).await;
        assert_eq!(p.part.as_deref(), Some("box"));
        assert!(p.saves.is_none() && p.snapshot.is_none() && p.error.is_none());
        let err = p.snapshot_error.unwrap_or_default();
        assert!(err.contains("the toy can't save today"), "{err}");
        teardown(fx).await;
    })
}

#[test]
fn benchmark_naming_an_unknown_part_is_a_tool_error() {
    run(async {
        let bench_args = ["--bench-tool", "--tool-part", "nowhere"];
        let root = packages::toy_world("badpart", &[], &bench_args);
        let fx = setup_with("badpart", std::slice::from_ref(&root.0), "toybench").await;
        let r = call(&fx, "resolver", "ring", json!({})).await;
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(
            r["content"][0]["text"],
            "the benchmark named an unknown part nowhere"
        );
        let p = payload(&fx, 0).await;
        assert!(p.part.is_none() && p.saves.is_none());
        teardown(fx).await;
    })
}
