use std::{
    future::Future,
    path::Path,
    sync::{Arc, LazyLock},
};

use arenad::{
    agents::{claude, claude::ClaudeDriver, codex, codex::CodexDriver, AgentRun, Driver, Mcp},
    app::{self, AppState, Config},
    exec::BranchCtx,
    model::*,
    store::Store,
    world::World,
};
use serde_json::json;

#[path = "support/app.rs"]
mod app_support;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn spec(model: Option<&str>) -> AgentSpec {
    AgentSpec {
        id: "triage".into(),
        name: "Triage".into(),
        kind: AgentKind::ClaudeCode,
        model: model.map(Into::into),
        prompt: "p".into(),
    }
}

fn args(c: &std::process::Command) -> Vec<String> {
    c.get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

const URL: &str = "http://127.0.0.1:1/mcp/b1/triage";

/// The MCP endpoint at `url`, named `server`.
fn mcp_named(server: &str, url: &str) -> Mcp {
    Mcp {
        server: server.into(),
        url: url.into(),
    }
}

/// The MCP endpoint at `URL`, named `bench`.
fn mcp() -> Mcp {
    mcp_named("bench", URL)
}

#[test]
fn claude_command_has_only_the_world_mcp() {
    let c = claude::command(&spec(None), &mcp(), "do it");
    let a = args(&c);
    assert_eq!(c.get_program(), "claude");
    assert!(a.contains(&"--strict-mcp-config".to_string()));
    let i = a.iter().position(|x| x == "--mcp-config").unwrap();
    let cfg: serde_json::Value = serde_json::from_str(&a[i + 1]).unwrap();
    assert_eq!(
        cfg,
        json!({"mcpServers": {"bench": {"type": "http", "url": URL}}})
    );
    let i = a.iter().position(|x| x == "--tools").unwrap();
    assert_eq!(a[i + 1], "");
    let i = a.iter().position(|x| x == "--allowedTools").unwrap();
    assert_eq!(a[i + 1], "mcp__bench__*");
    let i = a.iter().position(|x| x == "--model").unwrap();
    assert_eq!(a[i + 1], "sonnet");
    assert_eq!(a[0], "-p");
    assert_eq!(a[a.len() - 2..], ["--", "do it"]);
    assert!(a.contains(&"--no-session-persistence".to_string()));
    assert!(c.get_envs().any(|(k, v)| k == "PGPASSWORD" && v.is_none()));
    let c = claude::command(&spec(Some("opus")), &mcp(), "x");
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
    let failed = codex::parse(
        "{\"type\":\"turn.failed\",\"error\":{\"message\":\"usage limit\"}}\n",
        1,
    );
    assert_eq!(failed.error.as_deref(), Some("usage limit"));
}

#[test]
fn codex_command_sets_mcp_url() {
    let c = codex::command(&spec(Some("gpt-x")), &mcp(), Path::new("/tmp/w"), "do it");
    let a = args(&c);
    assert_eq!(c.get_program(), "codex");
    assert_eq!(a[0], "exec");
    assert_eq!(a[a.len() - 2..], ["--", "do it"]);
    assert!(a.contains(&format!("mcp_servers.bench.url=\"{URL}\"")));
    assert!(a.contains(&"approval_policy=\"never\"".to_string()));
    assert!(a.contains(&"mcp_servers.bench.default_tools_approval_mode=\"approve\"".to_string()));
    assert!(a.contains(&"--ignore-user-config".to_string()));
    assert!(a.contains(&"--ephemeral".to_string()));
    assert!(a.windows(2).any(|w| w == ["--disable", "apps"]));
    assert!(a.windows(2).any(|w| w == ["--disable", "plugins"]));
    assert!(a.windows(2).any(|w| w == ["--disable", "hooks"]));
    assert!(a.contains(&"web_search=\"disabled\"".to_string()));
    assert!(a.windows(2).any(|w| w == ["-m", "gpt-x"]));
    let removed: Vec<_> = c
        .get_envs()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    for k in [
        "ARENA_PG_URL",
        "DATABASE_URL",
        "PGPASSWORD",
        "OPENROUTER_API_KEY",
    ] {
        assert!(removed.contains(&k.to_string()), "{k} not scrubbed");
    }
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

/// A running toy world for the driver tests (CLI drivers never touch the world).
struct ToyWorld {
    world: Arc<World>,
    _hub: packages::TestHub,
    _root: packages::ToyRoot,
}

async fn toy_world(tag: &str) -> ToyWorld {
    let root = packages::toy_world(tag, &[], &[]);
    let hub = packages::TestHub::new(std::slice::from_ref(&root.0));
    let world = hub.start_world("toybench", "b1").await;
    ToyWorld {
        world,
        _hub: hub,
        _root: root,
    }
}

fn bare_run(toy: &ToyWorld, resume: Option<Vec<serde_json::Value>>) -> AgentRun {
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![spec(None)],
    };
    let branch = BranchCtx::new("b1".into(), toy.world.clone(), team, "task".into());
    AgentRun {
        branch: Arc::new(branch),
        spec: spec(None),
        input: "ticket".into(),
        resume,
    }
}

#[test]
fn resume_is_rejected() {
    run(async {
        let toy = toy_world("cliresume").await;
        let r = ClaudeDriver::new("http://x")
            .run(bare_run(&toy, Some(vec![])))
            .await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("CLI agents cannot resume mid-run"));
        let r = CodexDriver::new("http://x")
            .run(bare_run(&toy, Some(vec![])))
            .await;
        assert_eq!(r.error.as_deref(), Some("CLI agents cannot resume mid-run"));
    })
}

#[test]
fn driver_runs_binary_and_keeps_stderr_on_failure() {
    run(async {
        let toy = toy_world("clibinary").await;
        let mut d = ClaudeDriver::new("http://x");
        d.binary = fake("echo boom >&2; echo 'not json'; exit 3");
        let r = d.run(bare_run(&toy, None)).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("boom"));

        let mut d = CodexDriver::new("http://x");
        d.binary = fake("echo 'bad news' >&2; exit 2");
        let r = d.run(bare_run(&toy, None)).await;
        let e = r.error.unwrap();
        assert!(
            e.contains("exited with code 2") && e.contains("bad news"),
            "{e}"
        );

        let mut d = ClaudeDriver::new("http://x");
        d.binary = fake("cat <<'EOF'\n{\"is_error\":false,\"result\":\"hi\",\"total_cost_usd\":0.01,\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}\nEOF");
        let r = d.run(bare_run(&toy, None)).await;
        assert!(r.ok, "{:?}", r.error);
    })
}

/// A fake CLI that writes its arguments, one per line, to a fresh file: (the CLI, the file).
fn arg_recorder() -> (String, std::path::PathBuf) {
    let out = std::env::temp_dir().join(format!("fakeargs-{}", uuid::Uuid::new_v4().simple()));
    let script = format!("printf '%s\\n' \"$@\" > {}; exit 1", out.display());
    (fake(&script), out)
}

#[test]
fn drivers_name_the_mcp_server_after_the_benchmark() {
    run(async {
        let toy = toy_world("cliserver").await;
        let (binary, out) = arg_recorder();
        let mut d = ClaudeDriver::new("http://x");
        d.binary = binary.clone();
        d.run(bare_run(&toy, None)).await;
        let claude = std::fs::read_to_string(&out).unwrap_or_default();
        let mut d = CodexDriver::new("http://x");
        d.binary = binary.clone();
        d.run(bare_run(&toy, None)).await;
        let codex = std::fs::read_to_string(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&binary);
        assert!(claude.lines().any(|l| l == "mcp__toybench__*"), "{claude}");
        assert!(claude.contains(r#"{"mcpServers":{"toybench":"#), "{claude}");
        let url = "mcp_servers.toybench.url=\"http://x/mcp/b1/triage\"";
        assert!(codex.lines().any(|l| l == url), "{codex}");
    })
}

#[test]
fn driver_times_out() {
    run(async {
        let toy = toy_world("clitimeout").await;
        let mut d = CodexDriver::new("http://x");
        d.binary = fake("sleep 30");
        d.timeout_secs = 1;
        let r = d.run(bare_run(&toy, None)).await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("timed out after 1 s"));
        assert!(r.seconds < 10.0);
    })
}

// ---- live smoke tests (use the user's CLI sign-in; cost a few cents) ----

async fn live_setup(
    tag: &str,
) -> (
    Arc<Store>,
    pg::Cleanup,
    String,
    Arc<World>,
    String,
    packages::TestHub,
) {
    let (store, clean) = app_support::meta_store("cli", tag).await;
    let a = |id: &str| AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::ClaudeCode,
        model: None,
        prompt: "p".into(),
    };
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![a("triage"), a("resolver")],
    };
    let run_id = store.create_run("task", &team).await.unwrap();
    let branch = store
        .create_branch(&run_id, None, &team, &json!([]), None)
        .await
        .unwrap();
    let hub = packages::TestHub::new(&[]);
    let world = hub.start_world("retail", &branch).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mut cfg = Config::new();
    cfg.self_base = base.clone();
    let state = Arc::new(AppState::new(store.clone(), hub.hub.clone(), cfg));
    state.live.write().unwrap().insert(
        branch.clone(),
        Arc::new(BranchCtx::new(
            branch.clone(),
            world.clone(),
            team,
            "task".into(),
        )),
    );
    tokio::spawn(async move { axum::serve(listener, app::router(state)).await.unwrap() });
    (store, clean, branch, world, base, hub)
}

async fn live(tag: &str, mk: impl FnOnce(String) -> Box<dyn Driver>) {
    live_with(tag, mk, "Call the list_all_product_types tool exactly once, then reply with the word done. Do not call any other tool.", 1).await
}

async fn live_with(
    tag: &str,
    mk: impl FnOnce(String) -> Box<dyn Driver>,
    input: &str,
    want_calls: usize,
) {
    let (store, _clean, branch, world, base, _hub) = live_setup(tag).await;
    let driver = mk(base);
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![],
    };
    let ctx = Arc::new(BranchCtx::new(
        branch.clone(),
        world.clone(),
        team,
        "task".into(),
    ));
    let r = driver
        .run(AgentRun {
            branch: ctx,
            spec: AgentSpec {
                id: "triage".into(),
                name: "t".into(),
                kind: AgentKind::ClaudeCode,
                model: None,
                prompt: "You are a test agent. Use the retail MCP tools.".into(),
            },
            input: input.into(),
            resume: None,
        })
        .await;
    println!("RESULT: {r:?}");
    let evs = store.events(&branch).await.unwrap();
    for e in &evs {
        println!("EVENT: {:?} {:?} {}", e.kind, e.agent, e.payload);
    }
    drop(store);
    world.stop().await;
    assert!(r.ok, "{:?}", r.error);
    let calls: Vec<_> = evs
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .collect();
    assert_eq!(calls.len(), want_calls, "unexpected ToolCall count");
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

const PROBE: &str = "Do two things and report raw results. (1) List the exact name of EVERY tool, MCP server and plugin you can use, one per line, prefixed 'TOOL:'. (2) Run this shell command and print its raw output prefixed 'NET:': bash -c 'timeout 3 bash -c \"echo > /dev/tcp/127.0.0.1/5544\" && echo OPEN || echo CLOSED'. Call no retail tool.";

#[test]
#[ignore]
fn live_codex_isolation_probe() {
    run(async {
        let (store, _clean, branch, world, base, _hub) = live_setup("probe").await;
        let sp = AgentSpec {
            id: "triage".into(),
            name: "t".into(),
            kind: AgentKind::Codex,
            model: None,
            prompt: "".into(),
        };
        let wd = std::env::temp_dir().join(format!("probe-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&wd).unwrap();
        let endpoint = mcp_named("retail", &format!("{base}/mcp/{branch}/triage"));
        let mut cmd = codex::command(&sp, &endpoint, &wd, PROBE);
        let out = tokio::task::spawn_blocking(move || {
            cmd.stdin(std::process::Stdio::null()).output().unwrap()
        })
        .await
        .unwrap();
        let _ = std::fs::remove_dir_all(&wd);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let answer = stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["item"]["type"] == "agent_message")
            .filter_map(|v| v["item"]["text"].as_str().map(String::from))
            .last()
            .unwrap_or_default();
        println!("ANSWER:\n{answer}");
        drop(store);
        world.stop().await;
        for bad in [
            "codex_apps",
            "node_repl",
            "web__run",
            "browser",
            "figma",
            "runpod",
            "image_gen",
            "computer",
        ] {
            assert!(
                !answer.contains(bad),
                "unexpected tool/server `{bad}` visible"
            );
        }
        assert!(answer.contains("mcp__retail__list_all_product_types"));
        assert!(
            answer.contains("CLOSED") && !answer.contains("OPEN\n"),
            "db port reachable from sandbox?"
        );
    })
}
