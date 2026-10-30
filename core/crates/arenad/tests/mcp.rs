use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use arenad::{
    app::{self, AppState, Config},
    exec::BranchCtx,
    model::*,
    store::Store,
};
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use world::{
    dump,
    schema::load_base,
    snapshot::{Source, Worlds},
    testing,
};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

static WORLDS: OnceCell<Arc<Worlds>> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

async fn worlds() -> Arc<Worlds> {
    WORLDS
        .get_or_init(|| async {
            let w = Worlds::new(testing::admin());
            load_base(w.admin(), &data_dir()).await.unwrap();
            Arc::new(w)
        })
        .await
        .clone()
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec { id: id.into(), name: id.into(), kind: AgentKind::Open, model: None, prompt: "p".into() }
}

struct Fx {
    store: Arc<Store>,
    worlds: Arc<Worlds>,
    meta: String,
    branch: String,
    db: String,
    url: String,
}

async fn setup(tag: &str) -> Fx {
    let worlds = worlds().await;
    let meta = format!("arena_test_mcp_{}_{}", tag, &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let team = Team { id: "t".into(), name: "T".into(), agents: vec![agent("triage"), agent("resolver")] };
    let run_id = store.create_run("task", &team).await.unwrap();
    let branch = store.create_branch(&run_id, None, &team, &json!([]), None).await.unwrap();
    let db = worlds.new_branch(&format!("mcptest_{}", branch), Source::Base).await.unwrap();
    let state = Arc::new(AppState::new(
        store.clone(),
        worlds.clone(),
        Config::new(data_dir()),
    ));
    state
        .live
        .write()
        .unwrap()
        .insert(branch.clone(), Arc::new(BranchCtx::new(branch.clone(), db.clone(), team, "task".into())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app::router(state)).await.unwrap() });
    Fx { store, worlds, meta, branch, db, url: format!("http://127.0.0.1:{port}/mcp") }
}

async fn teardown(fx: Fx, extra: &[String]) {
    let Fx { store, worlds, meta, db, .. } = fx;
    drop(store);
    worlds.drop_db(&db).await.unwrap();
    for d in extra {
        worlds.drop_db(d).await.unwrap();
    }
    worlds.admin().drop_db(&meta).await.unwrap();
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
    let (s, v) = rpc(fx, agent, json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}})).await;
    assert_eq!(s, 200);
    v["result"].clone()
}

fn address_args() -> Value {
    json!({"user_id": "ethan_garcia_1261", "address1": "7 Mcp Road",
        "address2": "", "city": "New York", "state": "NY", "country": "USA", "zip": "10001"})
}

fn names(v: &Value) -> Vec<String> {
    v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn initialize_and_notification() {
    run(async {
        let fx = setup("init").await;
        let (s, v) = rpc(&fx, "triage", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})).await;
        assert_eq!(s, 200);
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(v["result"]["serverInfo"]["name"], "arena-retail");
        assert_eq!(v["result"]["capabilities"], json!({"tools":{}}));
        let (s, _) = rpc(&fx, "triage", json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        assert_eq!(s, 202);
        let (s, _) = rpc(&fx, "nobody", json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert_eq!(s, 404);
        teardown(fx, &[]).await;
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
        teardown(fx, &[]).await;
    })
}

#[test]
fn triage_write_is_refused_and_recorded() {
    run(async {
        let fx = setup("refuse").await;
        let r = call(&fx, "triage", "modify_user_address", address_args()).await;
        assert_eq!(r["isError"], true);
        let evs = fx.store.events(&fx.branch).await.unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, EventKind::ToolCall);
        assert_eq!(evs[0].agent.as_deref(), Some("triage"));
        let p: ToolCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        assert!(p.error.is_some());
        assert!(p.snapshot.is_none());
        // DB untouched.
        let before = {
            let b = fx.worlds.new_branch(&format!("mcpcmp_{}", fx.branch), Source::Base).await.unwrap();
            let d = fx.worlds.with_client(&b, async |c| dump(c).await).await.unwrap();
            fx.worlds.drop_db(&b).await.unwrap();
            d
        };
        let now = fx.worlds.with_client(&fx.db, async |c| dump(c).await).await.unwrap();
        assert_eq!(before, now);
        teardown(fx, &[]).await;
    })
}

#[test]
fn resolver_write_snapshots() {
    run(async {
        let fx = setup("snap").await;
        let r = call(&fx, "resolver", "modify_user_address", address_args()).await;
        assert_eq!(r["isError"], false, "{r}");
        assert!(r["structuredContent"].is_object());
        let evs = fx.store.events(&fx.branch).await.unwrap();
        assert_eq!(evs.len(), 1);
        let p: ToolCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        let snap = p.snapshot.expect("snapshot");
        let b2 = fx.worlds.new_branch(&format!("mcpfork_{}", fx.branch), Source::Snapshot(snap.clone())).await.unwrap();
        let (live, forked) = (
            fx.worlds.with_client(&fx.db, async |c| dump(c).await).await.unwrap(),
            fx.worlds.with_client(&b2, async |c| dump(c).await).await.unwrap(),
        );
        assert_eq!(live, forked);
        assert!(forked.to_string().contains("7 Mcp Road"));
        // A read does not snapshot.
        let r = call(&fx, "resolver", "get_user_details", json!({"user_id":"ethan_garcia_1261"})).await;
        assert_eq!(r["isError"], false);
        let evs = fx.store.events(&fx.branch).await.unwrap();
        let p: ToolCallPayload = serde_json::from_value(evs[1].payload.clone()).unwrap();
        assert!(p.snapshot.is_none());
        teardown(fx, &[b2, snap]).await;
    })
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
            let (s, v) = rpc(&fx, "resolver", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params})).await;
            assert_eq!(s, 200, "case {i}");
            assert_eq!(v["result"]["isError"], true, "case {i}: {v}");
            let evs = fx.store.events(&fx.branch).await.unwrap();
            assert_eq!(evs.len(), i + 1, "case {i}");
        }
        teardown(fx, &[]).await;
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
            vec![EventKind::ToolCall, EventKind::ToolCall, EventKind::Handoff, EventKind::ToolCall, EventKind::ToolCall]
        );
        let h: HandoffPayload = serde_json::from_value(evs[2].payload.clone()).unwrap();
        assert_eq!(h.note, "user wants X");
        assert_eq!(evs[2].agent.as_deref(), Some("triage"));
        teardown(fx, &[]).await;
    })
}

async fn seed_second_user(fx: &Fx) -> (String, String) {
    // Two distinct users from the data so the writes are independent.
    let ids: Vec<String> = fx
        .worlds
        .with_client(&fx.db, async |c| {
            let rows = c.query("SELECT id FROM users ORDER BY id LIMIT 2", &[]).await?;
            Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
        })
        .await
        .unwrap();
    (ids[0].clone(), ids[1].clone())
}

fn addr_for(user: &str, line: &str) -> Value {
    json!({"user_id": user, "address1": line, "address2": "", "city": "New York",
           "state": "NY", "country": "USA", "zip": "10001"})
}

#[test]
fn parallel_writes_snapshot_in_order() {
    run(async {
        let fx = setup("par").await;
        let (u1, u2) = seed_second_user(&fx).await;
        let (a, b) = tokio::join!(
            call(&fx, "resolver", "modify_user_address", addr_for(&u1, "1 Alpha Way")),
            call(&fx, "resolver", "modify_user_address", addr_for(&u2, "2 Beta Way")),
        );
        assert_eq!(a["isError"], false, "{a}");
        assert_eq!(b["isError"], false, "{b}");
        let evs = fx.store.events(&fx.branch).await.unwrap();
        assert_eq!(evs.len(), 2);
        assert!(evs[0].seq < evs[1].seq);
        let mut extra = vec![];
        let mut seen = vec![];
        for (i, e) in evs.iter().enumerate() {
            let p: ToolCallPayload = serde_json::from_value(e.payload.clone()).unwrap();
            let line = p.args["address1"].as_str().unwrap().to_string();
            let snap = p.snapshot.expect("snapshot");
            let d = fx.worlds.new_branch(&format!("mcppar{}_{}", i, fx.branch), Source::Snapshot(snap.clone())).await.unwrap();
            let dumped = fx.worlds.with_client(&d, async |c| dump(c).await).await.unwrap().to_string();
            seen.push(line.clone());
            for l in ["1 Alpha Way", "2 Beta Way"] {
                let expected = seen.iter().any(|s| s == l);
                assert_eq!(dumped.contains(l), expected, "event {i} snapshot, line {l}");
            }
            extra.extend([d, snap]);
        }
        teardown(fx, &extra).await;
    })
}

#[test]
fn failed_snapshot_is_recorded() {
    run(async {
        let fx = setup("snapfail").await;
        let exec = arenad::exec::ToolExec::new(fx.store.clone(), fx.worlds.clone());
        let ctx = BranchCtx::new(
            fx.branch.clone(),
            fx.db.clone(),
            Team { id: "t".into(), name: "T".into(), agents: vec![agent("resolver")] },
            "task".into(),
        );
        // An open session on the branch DB makes `CREATE DATABASE .. TEMPLATE` fail.
        let (_keep, conn) = tokio_postgres::connect(&fx.worlds.admin().db_url(&fx.db), tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(conn);
        let v = exec.run(&ctx, "resolver", "modify_user_address", address_args(), None).await;
        assert!(v.get("error").is_none(), "{v}");
        let evs = fx.store.events(&fx.branch).await.unwrap();
        let p: ToolCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        assert!(p.snapshot.is_none());
        assert!(p.snapshot_error.is_some());
        assert!(p.error.is_none());
        drop(_keep);
        teardown(fx, &[]).await;
    })
}
