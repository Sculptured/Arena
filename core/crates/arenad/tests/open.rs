#[path = "support/app.rs"]
mod app_support;
#[path = "support/fake_upstream.rs"]
mod fake_upstream;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use arenad::{
    agents::{open::OpenDriver, AgentRun, Driver, FailureKind},
    app::{self, AppState, Config},
    exec::BranchCtx,
    model::*,
    providers::{ProviderConfig, Secret},
};
use fake_upstream::FakeUpstream;
use packages::TestHub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn agent(id: &str, prompt: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("fake/model".into()),
        prompt: prompt.into(),
        provider: None,
        read_only: false,
    }
}

struct Fx {
    state: Arc<AppState>,
    ctx: Arc<BranchCtx>,
    _hub: TestHub,
    _clean: Cleanup,
}

/// Config that sends model calls to the fake `upstream` and names the arena's own address.
fn open_config(upstream: &FakeUpstream, self_base: String) -> Config {
    let mut config = Config::new();
    config.providers = ProviderConfig::with_upstream(&upstream.base_url);
    config.providers.openrouter.key = Some(Secret::new("sk-secret"));
    config.self_base = self_base;
    config
}

async fn setup(tag: &str, agents: Vec<AgentSpec>, upstream: &FakeUpstream) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("open", tag).await;
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents,
    };
    let run_id = store.create_run("task", &team).await.unwrap();
    let branch = store
        .create_branch(&run_id, None, &team, &json!([]), None)
        .await
        .unwrap();
    let world = hub.start_world("retail", &branch).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = open_config(upstream, format!("http://127.0.0.1:{port}"));
    let state = Arc::new(AppState::new(store, hub.hub.clone(), config));
    let ctx = Arc::new(BranchCtx::new(branch.clone(), world, team, "task".into()));
    state.live.write().unwrap().insert(branch, ctx.clone());
    let app = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fx {
        state,
        ctx,
        _hub: hub,
        _clean: clean,
    }
}

async fn teardown(fx: Fx) {
    fx.ctx.world.stop().await;
}

fn tool_msg(calls: Vec<(&str, &str, Value)>) -> Value {
    let tcs: Vec<Value> = calls
        .into_iter()
        .map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}))
        .collect();
    json!({"role": "assistant", "content": null, "tool_calls": tcs})
}

fn text_msg(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn address_args() -> Value {
    json!({"user_id": "ethan_garcia_1261", "address1": "7 Open Road",
        "address2": "", "city": "New York", "state": "NY", "country": "USA", "zip": "10001"})
}

/// The last message of a recorded upstream request.
fn last_message(req: &Value) -> Value {
    req["messages"].as_array().unwrap().last().unwrap().clone()
}

/// Whether the request offers the tool `name`.
fn offers_tool(req: &Value, name: &str) -> bool {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["function"]["name"] == name)
}

fn driver(fx: &Fx) -> OpenDriver {
    OpenDriver::new(fx.state.clone())
}

fn agent_run(fx: &Fx, idx: usize, resume: Option<Vec<Value>>) -> AgentRun {
    AgentRun {
        branch: fx.ctx.clone(),
        spec: fx.ctx.team.agents[idx].clone(),
        input: "Please help with my order.".into(),
        resume,
    }
}

async fn kinds(fx: &Fx) -> Vec<EventKind> {
    fx.state
        .store
        .events(&fx.ctx.branch_id)
        .await
        .unwrap()
        .iter()
        .map(|e| e.kind)
        .collect()
}

/// The first model call of the scripted run, as recorded: local provider, no charge, the solo
/// agent's prompt, the retail tools but no handoff. Returns its payload.
fn assert_first_model_call(ev: &Event) -> ModelCallPayload {
    let mc: ModelCallPayload = serde_json::from_value(ev.payload.clone()).unwrap();
    assert_eq!(mc.cost_usd, Some(0.0));
    assert_eq!(mc.cost_source.as_deref(), Some("local (no API charge)"));
    assert_eq!(mc.provider.as_deref(), Some("local:custom"));
    assert_eq!(mc.provider_response, None);
    assert_eq!(mc.request["model"], "fake/model");
    assert_eq!(
        mc.request["messages"][0],
        json!({"role": "system", "content": "You are the solo agent."})
    );
    assert_eq!(mc.request["messages"][1]["role"], "user");
    assert!(offers_tool(&mc.request, "get_order_details"));
    assert!(!offers_tool(&mc.request, "handoff"));
    assert_eq!(
        mc.response["choices"][0]["message"]["tool_calls"][0]["id"],
        "c1"
    );
    assert_eq!(ev.agent.as_deref(), Some("solo"));
    mc
}

/// The upstream saw exactly what was recorded, and the first tool result went back.
fn assert_upstream_saw_recorded(up: &FakeUpstream, mc: &ModelCallPayload) {
    let reqs = up.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0], mc.request);
    assert!(up.auth_headers().iter().all(Option::is_none));
    let last = last_message(&reqs[1]);
    assert_eq!(last["role"], "tool");
    assert_eq!(last["tool_call_id"], "c1");
}

/// A read, a write and a final answer.
async fn scripted_upstream() -> FakeUpstream {
    FakeUpstream::start(vec![
        tool_msg(vec![(
            "c1",
            "get_order_details",
            json!({"order_id": "#W0000000"}),
        )]),
        tool_msg(vec![("c2", "modify_user_address", address_args())]),
        text_msg("All done."),
    ])
    .await
}

/// The token totals of the scripted run; the upstream override is the local provider
/// `local:custom`, so no API charge.
fn assert_local_cost(r: &arenad::agents::AgentResult) {
    assert_eq!(
        r.tokens,
        json!({"prompt_tokens": 30, "completion_tokens": 15, "total_tokens": 45})
    );
    assert_eq!(r.cost_usd, Some(0.0));
    assert_eq!(r.cost_source.as_deref(), Some("local (no API charge)"));
}

#[test]
fn open_agent_runs_scripted_tools_and_records_model_calls() {
    run(async {
        let up = scripted_upstream().await;
        let fx = setup(
            "scripted",
            vec![agent("solo", "You are the solo agent.")],
            &up,
        )
        .await;
        let d: Arc<dyn Driver> = Arc::new(driver(&fx));
        let r = d.run(agent_run(&fx, 0, None)).await;
        assert!(r.ok, "{:?}", r.error);
        assert_local_cost(&r);
        assert_eq!(
            kinds(&fx).await,
            vec![
                EventKind::ModelCall,
                EventKind::ToolCall,
                EventKind::ModelCall,
                EventKind::ToolCall,
                EventKind::ModelCall
            ]
        );
        let evs = fx.state.store.events(&fx.ctx.branch_id).await.unwrap();
        let mc = assert_first_model_call(&evs[0]);
        assert_upstream_saw_recorded(&up, &mc);
        let tc: ToolCallPayload = serde_json::from_value(evs[3].payload.clone()).unwrap();
        assert!(tc.snapshot.is_some());
        assert_eq!(tc.tool_call_id.as_deref(), Some("c2"));
        teardown(fx).await;
    })
}

#[test]
fn upstream_error_is_recorded_and_agent_fails() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"_status": 500})]).await;
        let fx = setup("err", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        assert!(r.error.is_some());
        let evs = fx.state.store.events(&fx.ctx.branch_id).await.unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, EventKind::ModelCall);
        let mc: ModelCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        assert!(mc.response["error"].is_string() || mc.response["error"].is_object());
        assert_eq!(mc.cost_usd, None);
        assert_eq!(mc.cost_source, None);
        teardown(fx).await;
    })
}

#[test]
fn forty_call_cap_stops_the_loop() {
    run(async {
        let up = FakeUpstream::start(vec![tool_msg(vec![(
            "c",
            "get_order_details",
            json!({"order_id": "#W0000000"}),
        )])])
        .await;
        let fx = setup("cap", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("40"));
        let k = kinds(&fx).await;
        assert_eq!(k.iter().filter(|k| **k == EventKind::ModelCall).count(), 40);
        assert_eq!(up.requests().len(), 40);
        teardown(fx).await;
    })
}

/// The agent hands off at once.
async fn handoff_upstream() -> FakeUpstream {
    FakeUpstream::start(vec![
        tool_msg(vec![(
            "h1",
            "handoff",
            json!({"note": "customer wants a new address"}),
        )]),
        text_msg("unused"),
    ])
    .await
}

/// A recorded conversation under a system prompt the fork replaces.
fn old_history() -> Vec<Value> {
    vec![
        json!({"role": "system", "content": "OLD PROMPT"}),
        json!({"role": "user", "content": "earlier input"}),
    ]
}

/// The one upstream request carries the new system prompt over the resumed history, offers the
/// handoff tool, and has no OpenRouter usage flag (it goes to OpenRouter alone).
fn assert_resumed_request(reqs: &[Value]) {
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0]["messages"][0],
        json!({"role": "system", "content": "NEW PROMPT"})
    );
    assert_eq!(reqs[0]["messages"][1]["content"], "earlier input");
    assert!(offers_tool(&reqs[0], "handoff"));
    assert_eq!(reqs[0].get("usage"), None);
}

#[test]
fn handoff_stops_loop_and_resume_replaces_system_prompt() {
    run(async {
        let up = handoff_upstream().await;
        let fx = setup(
            "handoff",
            vec![agent("triage", "NEW PROMPT"), agent("resolver", "r")],
            &up,
        )
        .await;
        let r = driver(&fx)
            .run(agent_run(&fx, 0, Some(old_history())))
            .await;
        assert!(r.ok, "{:?}", r.error);
        assert_eq!(
            kinds(&fx).await,
            vec![EventKind::ModelCall, EventKind::Handoff]
        );
        assert_eq!(
            fx.ctx.handoff_note("triage").as_deref(),
            Some("customer wants a new address")
        );
        assert_resumed_request(&up.requests());
        teardown(fx).await;
    })
}

#[test]
fn malformed_tool_arguments_are_not_run() {
    run(async {
        let bad = json!({"role": "assistant", "content": null, "tool_calls": [
            {"id": "b1", "type": "function", "function": {"name": "get_order_details", "arguments": "{\"order_id\": \"#W00"}}]});
        let up = FakeUpstream::start(vec![bad, text_msg("sorry")]).await;
        let fx = setup("badargs", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(r.ok, "{:?}", r.error);
        assert_eq!(
            kinds(&fx).await,
            vec![EventKind::ModelCall, EventKind::ModelCall]
        );
        let reqs = up.requests();
        let last = last_message(&reqs[1]);
        assert_eq!(last["tool_call_id"], "b1");
        assert!(last["content"].as_str().unwrap().contains("not valid JSON"));
        teardown(fx).await;
    })
}

#[test]
fn truncated_final_message_fails_the_agent() {
    run(async {
        let up = FakeUpstream::start(vec![
            json!({"role": "assistant", "content": "partial", "_finish_reason": "length"}),
        ])
        .await;
        let fx = setup("cut", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("model output was cut off"));
        assert_eq!(r.failure, Some(FailureKind::Model));
        teardown(fx).await;
    })
}

/// The OpenRouter key `setup` sets never reaches the custom upstream either.
#[test]
fn gateway_never_forwards_authorization_to_non_openrouter_upstream() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("hi")]).await;
        let fx = setup("auth", vec![agent("solo", "p")], &up).await;
        assert!(fx.state.config.open_available());
        let url = format!(
            "{}/gateway/{}/solo/v1/chat/completions",
            fx.state.config.self_base, fx.ctx.branch_id
        );
        let r = reqwest::Client::new()
            .post(url)
            .header("Authorization", "Bearer incoming-secret")
            .json(&json!({"model": "fake/model", "messages": [{"role": "user", "content": "x"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(up.auth_headers(), vec![None]);
        teardown(fx).await;
    })
}

#[test]
fn model_call_cap_is_a_model_failure() {
    run(async {
        let call = tool_msg(vec![(
            "c",
            "get_order_details",
            json!({"order_id": "#W0000000"}),
        )]);
        let up = FakeUpstream::start(vec![call; arenad::agents::open::MAX_MODEL_CALLS]).await;
        let fx = setup("cap", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        assert_eq!(r.failure, Some(FailureKind::Model));
        assert!(r.error.unwrap().contains("exceeded"));
        teardown(fx).await;
    })
}

#[test]
fn upstream_error_is_an_infra_failure() {
    run(async {
        let up = FakeUpstream::start(vec![]).await;
        let fx = setup("upfail", vec![agent("solo", "p")], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        assert_eq!(r.failure, None);
        teardown(fx).await;
    })
}

/// Only OpenRouter has a default model (`tests/providers.rs` checks it is sent): an agent on
/// another provider that names none fails before any call.
#[test]
fn missing_model_without_a_provider_default_fails_before_calling() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("hi")]).await;
        let mut a = agent("solo", "p");
        a.model = None;
        let fx = setup("defmodel", vec![a], &up).await;
        let r = driver(&fx).run(agent_run(&fx, 0, None)).await;
        assert!(!r.ok);
        let error = r.error.unwrap();
        assert!(
            error.contains("names no model") && error.contains("local:custom"),
            "{error}"
        );
        assert!(up.requests().is_empty());
        teardown(fx).await;
    })
}

#[test]
fn gateway_refuses_unknown_branch_or_agent_without_calling_upstream() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("hi")]).await;
        let fx = setup("gw404", vec![agent("solo", "p")], &up).await;
        let body = json!({"model": "m", "messages": []});
        for (b, a) in [("nobranch", "solo"), (fx.ctx.branch_id.as_str(), "nobody")] {
            let url = format!(
                "{}/gateway/{}/{}/v1/chat/completions",
                fx.state.config.self_base, b, a
            );
            let r = reqwest::Client::new()
                .post(url)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 404);
            let v: Value = r.json().await.unwrap();
            assert!(v["error"].is_string());
        }
        assert!(up.requests().is_empty());
        teardown(fx).await;
    })
}

/// A provider the configuration doesn't know is recorded as a failed call, with no provider.
#[test]
fn an_unknown_provider_is_recorded_as_a_failed_call() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("hi")]).await;
        let mut a = agent("solo", "p");
        a.provider = Some("local:gone".into());
        let fx = setup("gone", vec![a], &up).await;
        let url = format!(
            "{}/gateway/{}/solo/v1/chat/completions",
            fx.state.config.self_base, fx.ctx.branch_id
        );
        let body = json!({"model": "m", "messages": [{"role": "user", "content": "x"}]});
        let r = reqwest::Client::new()
            .post(url)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 502);
        let evs = fx.state.store.events(&fx.ctx.branch_id).await.unwrap();
        assert_eq!(evs.len(), 1);
        let mc: ModelCallPayload = serde_json::from_value(evs[0].payload.clone()).unwrap();
        let error = mc.response["error"].as_str().unwrap();
        assert!(error.contains("unknown provider \"local:gone\""), "{error}");
        assert_eq!((mc.provider, mc.cost_usd), (None, None));
        assert!(up.requests().is_empty());
        teardown(fx).await;
    })
}
