use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use arenad::{
    agents::{claude, codex, claude::ClaudeDriver, codex::CodexDriver, AgentRun, Driver},
    app::{self, AppState, Config},
    exec::BranchCtx,
    model::*,
    store::Store,
};
use serde_json::json;
use tokio::sync::OnceCell;
use world::{
    schema::load_base,
    snapshot::{Source, Worlds},
    testing,
};

static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn spec(model: Option<&str>) -> AgentSpec {
    AgentSpec { id: "triage".into(), name: "Triage".into(), kind: AgentKind::ClaudeCode, model: model.map(Into::into), prompt: "p".into() }
}

fn args(c: &std::process::Command) -> Vec<String> {
    c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

const URL: &str = "http://127.0.0.1:1/mcp/b1/triage";

#[test]
fn claude_command_has_only_retail_mcp() {
    let c = claude::command(&spec(None), URL, "do it");
    let a = args(&c);
    assert_eq!(c.get_program(), "claude");
    assert!(a.contains(&"--strict-mcp-config".to_string()));
    let i = a.iter().position(|x| x == "--mcp-config").unwrap();
    let cfg: serde_json::Value = serde_json::from_str(&a[i + 1]).unwrap();
    assert_eq!(cfg, json!({"mcpServers": {"retail": {"type": "http", "url": URL}}}));
    let i = a.iter().position(|x| x == "--tools").unwrap();
    assert_eq!(a[i + 1], "");
    let i = a.iter().position(|x| x == "--allowedTools").unwrap();
    assert_eq!(a[i + 1], "mcp__retail__*");
    let i = a.iter().position(|x| x == "--model").unwrap();
    assert_eq!(a[i + 1], "sonnet");
    assert_eq!(a[1], "do it");
    let c = claude::command(&spec(Some("opus")), URL, "x");
    assert!(args(&c).contains(&"opus".to_string()));
}

#[test]
fn claude_parse_ok_and_error() {
    let ok = claude::parse(&fixture("claude_ok.json"));
    assert!(ok.ok);
    assert_eq!(ok.error, None);
    assert_eq!(ok.cost_source.as_deref(), Some("api_equivalent_estimate"));
    assert!((ok.cost_usd.unwrap() - 0.0735072).abs() < 1e-9);
    assert_eq!(ok.tokens["prompt_tokens"], 16 + 11230 + 68352);
    assert_eq!(ok.tokens["completion_tokens"], 2172);
    assert_eq!(ok.tokens["total_tokens"], 16 + 11230 + 68352 + 2172);

    let err = claude::parse(&fixture("claude_error.json"));
    assert!(!err.ok);
    assert!(err.error.unwrap().contains("hit your 5-hour limit"));

    let junk = claude::parse("not json");
    assert!(!junk.ok);
}

#[test]
fn codex_parse_tokens() {
    let r = codex::parse(&fixture("codex_ok.jsonl"), 0);
    assert!(r.ok);
    assert_eq!(r.cost_usd, None);
    assert_eq!(r.tokens["prompt_tokens"], 241348);
    assert_eq!(r.tokens["completion_tokens"], 913);
    assert_eq!(r.tokens["total_tokens"], 241348 + 913);
    let bad = codex::parse(&fixture("codex_ok.jsonl"), 1);
    assert!(!bad.ok);
    let none = codex::parse("", 0);
    assert!(!none.ok);
    let failed = codex::parse("{\"type\":\"turn.failed\",\"error\":{\"message\":\"usage limit\"}}\n", 1);
    assert_eq!(failed.error.as_deref(), Some("usage limit"));
}

#[test]
fn codex_command_sets_mcp_url() {
    let c = codex::command(&spec(Some("gpt-x")), URL, Path::new("/tmp/w"), "do it");
    let a = args(&c);
    assert_eq!(c.get_program(), "codex");
    assert_eq!(a[0], "exec");
    assert!(a.contains(&format!("mcp_servers.retail.url=\"{URL}\"")));
    assert!(a.contains(&"approval_policy=\"never\"".to_string()));
    assert!(a.windows(2).any(|w| w == ["-m", "gpt-x"]));
    assert!(a.windows(2).any(|w| w == ["-C", "/tmp/w"]));
    assert_eq!(a.last().unwrap(), "do it");
}

// ---- driver behaviour with fake binaries ----

fn fake(script: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let p = std::env::temp_dir().join(format!("fakecli-{}.sh", uuid::Uuid::new_v4().simple()));
    std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

fn bare_run(resume: Option<Vec<serde_json::Value>>) -> AgentRun {
    let team = Team { id: "t".into(), name: "T".into(), agents: vec![spec(None)] };
    AgentRun {
        branch: Arc::new(BranchCtx::new("b1".into(), "db".into(), team, "task".into())),
        spec: spec(None),
        input: "ticket".into(),
        resume,
    }
}

#[test]
fn resume_is_rejected() {
    run(async {
        let r = ClaudeDriver::new("http://x").run(bare_run(Some(vec![]))).await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("CLI agents cannot resume mid-run"));
        let r = CodexDriver::new("http://x").run(bare_run(Some(vec![]))).await;
        assert_eq!(r.error.as_deref(), Some("CLI agents cannot resume mid-run"));
    })
}

#[test]
fn driver_runs_binary_and_keeps_stderr_on_failure() {
    run(async {
        let mut d = ClaudeDriver::new("http://x");
        d.binary = fake("echo boom >&2; echo 'not json'; exit 3");
        let r = d.run(bare_run(None)).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("boom"));

        let mut d = CodexDriver::new("http://x");
        d.binary = fake("echo 'bad news' >&2; exit 2");
        let r = d.run(bare_run(None)).await;
        let e = r.error.unwrap();
        assert!(e.contains("exited with code 2") && e.contains("bad news"), "{e}");

        let mut d = ClaudeDriver::new("http://x");
        d.binary = fake("cat <<'EOF'\n{\"is_error\":false,\"result\":\"hi\",\"total_cost_usd\":0.01,\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}\nEOF");
        let r = d.run(bare_run(None)).await;
        assert!(r.ok, "{:?}", r.error);
    })
}

#[test]
fn driver_times_out() {
    run(async {
        let mut d = CodexDriver::new("http://x");
        d.binary = fake("sleep 30");
        d.timeout_secs = 1;
        let r = d.run(bare_run(None)).await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("timed out after 1 s"));
        assert!(r.seconds < 10.0);
    })
}

// ---- live smoke tests (use the user's CLI sign-in; cost a few cents) ----

static WORLDS: OnceCell<Arc<Worlds>> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

async fn live_setup(tag: &str) -> (Arc<Store>, Arc<Worlds>, String, String, String, String) {
    let worlds = WORLDS
        .get_or_init(|| async {
            let w = Worlds::new(testing::admin());
            load_base(w.admin(), &data_dir()).await.unwrap();
            Arc::new(w)
        })
        .await
        .clone();
    let meta = format!("arena_test_cli_{}_{}", tag, &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let a = |id: &str| AgentSpec { id: id.into(), name: id.into(), kind: AgentKind::ClaudeCode, model: None, prompt: "p".into() };
    let team = Team { id: "t".into(), name: "T".into(), agents: vec![a("triage"), a("resolver")] };
    let run_id = store.create_run("task", &team).await.unwrap();
    let branch = store.create_branch(&run_id, None, &team, &json!([]), None).await.unwrap();
    let db = worlds.new_branch(&format!("clitest_{}", branch), Source::Base).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mut cfg = Config::new(data_dir());
    cfg.self_base = base.clone();
    let state = Arc::new(AppState::new(store.clone(), worlds.clone(), cfg));
    state
        .live
        .write()
        .unwrap()
        .insert(branch.clone(), Arc::new(BranchCtx::new(branch.clone(), db.clone(), team, "task".into())));
    tokio::spawn(async move { axum::serve(listener, app::router(state)).await.unwrap() });
    (store, worlds, meta, branch, db, base)
}

async fn live(tag: &str, mk: impl FnOnce(String) -> Box<dyn Driver>) {
    let (store, worlds, meta, branch, db, base) = live_setup(tag).await;
    let driver = mk(base);
    let team = Team { id: "t".into(), name: "T".into(), agents: vec![] };
    let ctx = Arc::new(BranchCtx::new(branch.clone(), db.clone(), team, "task".into()));
    let r = driver
        .run(AgentRun {
            branch: ctx,
            spec: AgentSpec { id: "triage".into(), name: "t".into(), kind: AgentKind::ClaudeCode, model: None,
                prompt: "You are a test agent. Use the retail MCP tools.".into() },
            input: "Call the list_all_product_types tool exactly once, then reply with the word done. Do not call any other tool.".into(),
            resume: None,
        })
        .await;
    println!("RESULT: {r:?}");
    let evs = store.events(&branch).await.unwrap();
    for e in &evs {
        println!("EVENT: {:?} {:?} {}", e.kind, e.agent, e.payload);
    }
    drop(store);
    worlds.drop_db(&db).await.unwrap();
    worlds.admin().drop_db(&meta).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    let calls: Vec<_> = evs.iter().filter(|e| e.kind == EventKind::ToolCall).collect();
    assert_eq!(calls.len(), 1, "expected exactly one ToolCall");
    assert_eq!(calls[0].agent.as_deref(), Some("triage"));
}

#[test]
#[ignore]
fn live_claude_lists_tools() {
    run(live("claude", |b| Box::new(ClaudeDriver::new(b))))
}

#[test]
#[ignore]
fn live_codex_lists_tools() {
    run(live("codex", |b| Box::new(CodexDriver::new(b))))
}

