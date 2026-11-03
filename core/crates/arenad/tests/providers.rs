//! Model providers end to end: each agent's calls go to its own provider, with that provider's
//! key and request shape and cost label; `/api/providers` and `/api/health` report them; a fork
//! can move an agent to another provider. Every upstream is a fake server on 127.0.0.1.

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
    time::{Duration, Instant},
};

use arenad::{
    app::AppState,
    model::*,
    providers::{Endpoint, LocalEndpoint, ProviderConfig, ProviderKind, Secret},
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

const OR_KEY: &str = "sk-or-test-key";
const OAI_KEY: &str = "sk-oai-test-key";
const LOCAL_KEY: &str = "sk-local-test-key";

fn agent(id: &str, provider: Option<&str>, model: Option<&str>) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: model.map(str::to_string),
        prompt: format!("You are {id}."),
        provider: provider.map(str::to_string),
    }
}

fn team(agents: Vec<AgentSpec>) -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents,
    }
}

fn local(name: &str, base: &str, key: Option<&str>) -> LocalEndpoint {
    LocalEndpoint {
        name: name.into(),
        endpoint: Endpoint::new(base, key.map(Secret::new)),
    }
}

fn text_msg(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn tool_msg(id: &str, name: &str, args: Value) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": [
        {"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]})
}

fn handoff_msg() -> Value {
    tool_msg("h", "handoff", json!({"note": "over to you"}))
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    http: reqwest::Client,
    _hub: TestHub,
    _clean: Cleanup,
}

/// An app whose providers are exactly `providers` (nothing from the environment).
async fn setup(tag: &str, providers: ProviderConfig) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("prov", tag).await;
    let (state, base) = app_support::open_app(store, hub.hub.clone(), "http://127.0.0.1:9", |c| {
        c.providers = providers;
    })
    .await;
    Fx {
        state,
        base,
        http: reqwest::Client::new(),
        _hub: hub,
        _clean: clean,
    }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(self.http.get(format!("{}{path}", self.base))).await
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let req = self.http.post(format!("{}{path}", self.base)).json(&body);
        app_support::send(req).await
    }

    /// Starts a run of task 27 with `team`; the response's status and body.
    async fn start(&self, team: &Team) -> (u16, Value) {
        self.post("/api/runs", json!({"task_id": "27", "team": team}))
            .await
    }

    async fn wait(&self, branch: &str) -> Value {
        for _ in 0..600 {
            let (_, b) = self.get(&format!("/api/branches/{branch}")).await;
            if b["status"] != "running" {
                return b;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("branch still running");
    }

    async fn run_to_end(&self, team: &Team) -> Value {
        let (st, v) = self.start(team).await;
        assert_eq!(st, 200, "{v}");
        self.wait(v["branch_id"].as_str().unwrap()).await
    }
}

/// The payloads of `kind` events by `agent`.
fn payloads(branch: &Value, kind: &str, agent: &str) -> Vec<Value> {
    branch["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind && e["agent"] == agent)
        .map(|e| e["payload"].clone())
        .collect()
}

fn bearer(key: &str) -> Option<String> {
    Some(format!("Bearer {key}"))
}

/// Four agents, each on its own fake provider.
struct Four {
    router: FakeUpstream,
    openai: FakeUpstream,
    plain: FakeUpstream,
    keyed: FakeUpstream,
}

impl Four {
    async fn start() -> Four {
        Four {
            router: FakeUpstream::start(vec![handoff_msg()]).await,
            openai: FakeUpstream::start(vec![handoff_msg()]).await,
            plain: FakeUpstream::start(vec![handoff_msg()]).await,
            keyed: FakeUpstream::start(vec![text_msg("Done.")]).await,
        }
    }

    fn providers(&self) -> ProviderConfig {
        ProviderConfig {
            openrouter: Endpoint::new(&self.router.base_url, Some(Secret::new(OR_KEY))),
            openai: Endpoint::new(&self.openai.base_url, Some(Secret::new(OAI_KEY))),
            locals: vec![
                local("plain", &self.plain.base_url, None),
                local("keyed", &self.keyed.base_url, Some(LOCAL_KEY)),
            ],
            ..ProviderConfig::default()
        }
    }

    fn team() -> Team {
        team(vec![
            agent("router", None, None),
            agent("oai", Some("openai"), Some("gpt-test")),
            agent("plain", Some("local:plain"), Some("llama-test")),
            agent("keyed", Some("local:keyed"), Some("qwen-test")),
        ])
    }
}

/// The one request `u` got: its model, its usage field, and its Authorization header.
fn only_request(u: &FakeUpstream) -> (Value, Option<Value>, Option<String>) {
    let reqs = u.requests();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    let auth = u.auth_headers().pop().unwrap();
    (
        reqs[0]["model"].clone(),
        reqs[0].get("usage").cloned(),
        auth,
    )
}

/// Each upstream got its own agent's call: the right key (or none), and OpenRouter alone the
/// usage flag.
fn assert_requests(f: &Four) {
    let usage = Some(json!({"include": true}));
    let expect = [
        (
            &f.router,
            "deepseek/deepseek-chat-v3.1",
            usage,
            bearer(OR_KEY),
        ),
        (&f.openai, "gpt-test", None, bearer(OAI_KEY)),
        (&f.plain, "llama-test", None, None),
        (&f.keyed, "qwen-test", None, bearer(LOCAL_KEY)),
    ];
    for (up, model, usage, auth) in expect {
        assert_eq!(only_request(up), (json!(model), usage, auth), "{model}");
    }
}

const LOCAL_COST: &str = "local (no API charge)";

/// `agent`'s one model call names `provider` and carries `cost` and `source`; so does its end.
fn assert_agent_record(b: &Value, agent: &str, provider: &str, cost: Value, source: Value) {
    let calls = payloads(b, "model_call", agent);
    assert_eq!(calls.len(), 1, "{agent}");
    let mc: ModelCallPayload = serde_json::from_value(calls[0].clone()).unwrap();
    assert_eq!(mc.provider.as_deref(), Some(provider), "{agent}");
    let recorded = (json!(mc.cost_usd), json!(mc.cost_source));
    assert_eq!(recorded, (cost.clone(), source.clone()), "{agent}");
    // Chat-completions answers are canonical; the canonical request is what is recorded.
    assert_eq!(mc.provider_response, None, "{agent}");
    assert_eq!(mc.request.get("usage"), None, "{agent}");
    let end = &payloads(b, "agent_end", agent)[0];
    assert_eq!(
        (&end["cost_usd"], &end["cost_source"]),
        (&cost, &source),
        "{agent}"
    );
}

fn assert_recorded(b: &Value) {
    assert_agent_record(b, "router", "openrouter", json!(0.001), json!("openrouter"));
    assert_agent_record(b, "oai", "openai", Value::Null, Value::Null);
    assert_agent_record(b, "plain", "local:plain", json!(0.0), json!(LOCAL_COST));
    assert_agent_record(b, "keyed", "local:keyed", json!(0.0), json!(LOCAL_COST));
    let start = &payloads(b, "agent_start", "oai")[0];
    assert_eq!(start["provider"], "openai");
    assert_eq!(
        payloads(b, "agent_start", "router")[0].get("provider"),
        None
    );
}

#[test]
fn each_agent_calls_its_own_provider() {
    run(async {
        let four = Four::start().await;
        let fx = setup("route", four.providers()).await;
        let b = fx.run_to_end(&Four::team()).await;
        assert_ne!(b["status"], "infra_error", "{b}");
        assert_requests(&four);
        assert_recorded(&b);
        // No key is in the record, nor in what Debug prints of the configuration.
        let recorded = b.to_string();
        let config = format!("{:?}", fx.state.config);
        for key in [OR_KEY, OAI_KEY, LOCAL_KEY] {
            assert!(!recorded.contains(key), "{key} recorded");
            assert!(!config.contains(key), "{key} in Debug");
        }
    })
}

/// A server that accepts connections and never answers.
async fn slow_server() -> String {
    let (listener, base) = app_support::bind().await;
    tokio::spawn(async move {
        let mut held = vec![];
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    base
}

/// The base of a port nothing listens on.
async fn down_server() -> String {
    let (listener, base) = app_support::bind().await;
    drop(listener);
    base
}

async fn listing_providers(up: &FakeUpstream) -> ProviderConfig {
    ProviderConfig {
        openai: Endpoint::new(&up.base_url, Some(Secret::new(OAI_KEY))),
        locals: vec![
            local("up", &up.base_url, None),
            local("slow1", &slow_server().await, None),
            local("slow2", &slow_server().await, None),
            local("down", &down_server().await, None),
        ],
        ..ProviderConfig::default()
    }
}

fn by_id<'a>(list: &'a Value, id: &str) -> &'a Value {
    let found = list.as_array().unwrap().iter().find(|p| p["id"] == id);
    found.unwrap_or_else(|| panic!("no {id} in {list}"))
}

fn assert_listing(list: &Value) {
    let models = json!(["fake/model", "fake/other"]);
    let up = by_id(list, "local:up");
    assert_eq!(
        (&up["available"], &up["models"], &up["kind"]),
        (&json!(true), &models, &json!("local"))
    );
    assert_eq!(by_id(list, "openai")["models"], models);
    let or = by_id(list, "openrouter");
    assert_eq!(
        (&or["available"], &or["why"]),
        (&json!(false), &json!("OPENROUTER_API_KEY is not set"))
    );
    let an = by_id(list, "anthropic");
    assert_eq!(
        (&an["available"], &an["why"]),
        (&json!(false), &json!("Anthropic support is not built yet"))
    );
    for id in ["local:slow1", "local:slow2"] {
        let s = by_id(list, id);
        assert_eq!(s["available"], false, "{s}");
        assert!(s["why"].as_str().unwrap().contains("within 3 s"), "{s}");
    }
    let down = by_id(list, "local:down");
    assert_eq!(down["available"], false);
    assert!(
        down["why"].as_str().unwrap().contains("did not answer"),
        "{down}"
    );
}

#[test]
fn providers_route_reports_availability_and_models_within_the_bound() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup("list", listing_providers(&up).await).await;
        let t = Instant::now();
        let (st, list) = fx.get("/api/providers").await;
        let took = t.elapsed();
        assert_eq!(st, 200, "{list}");
        // Two servers that never answer, asked at once, each cut off at 3 s.
        assert!(took < Duration::from_millis(5500), "took {took:?}");
        assert_listing(&list);
        // OpenAI's list was asked with its key, the keyless local server's without one.
        assert_eq!(up.list_auth_headers(), vec![None, bearer(OAI_KEY)]);
        // Health answers from the configuration alone, at once.
        let t = Instant::now();
        let (_, h) = fx.get("/api/health").await;
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(h["open_available"], true);
        let summary = by_id(&h["providers"], "local:slow1");
        assert_eq!(
            (&summary["available"], summary.get("models")),
            (&json!(true), None)
        );
        assert_eq!(by_id(&h["providers"], "openrouter")["available"], false);
    })
}

/// (provider, model, why a run of an agent on them is refused).
const REFUSED: [(Option<&str>, Option<&str>, &str); 5] = [
    (
        Some("openrouter"),
        Some("m"),
        "open mode needs OPENROUTER_API_KEY",
    ),
    (None, Some("m"), "open mode needs OPENROUTER_API_KEY"),
    (Some("openai"), Some("m"), "open mode needs OPENAI_API_KEY"),
    (
        Some("anthropic"),
        Some("m"),
        "Anthropic support is not built yet",
    ),
    (
        Some("local:up"),
        None,
        "provider local:up has no default model: name one",
    ),
];

#[test]
fn runs_are_refused_on_providers_that_cannot_take_them() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let providers = ProviderConfig {
            locals: vec![local("up", &up.base_url, None)],
            ..ProviderConfig::default()
        };
        let fx = setup("refuse", providers).await;
        for (provider, model, why) in REFUSED {
            let (st, v) = fx.start(&team(vec![agent("a", provider, model)])).await;
            assert_eq!((st, v["error"].as_str()), (400, Some(why)), "{provider:?}");
        }
        let (st, v) = fx
            .start(&team(vec![agent("a", Some("local:nope"), Some("m"))]))
            .await;
        assert_eq!(st, 400);
        let error = v["error"].as_str().unwrap();
        assert!(error.contains("unknown provider \"local:nope\""), "{v}");
        assert!(up.requests().is_empty());
    })
}

/// The seq of the first model call of a branch.
fn first_model_call(b: &Value) -> i64 {
    let evs = b["events"].as_array().unwrap();
    let mc = evs.iter().find(|e| e["kind"] == "model_call").unwrap();
    mc["seq"].as_i64().unwrap()
}

/// The fork of `branch` at `at` that `changes` asks for.
fn fork_body(at: i64, changes: Value) -> Value {
    json!({"at_seq": at, "changes": {"solo": changes}})
}

/// Refused before anything starts: no key, or no model on a provider without a default.
async fn assert_fork_refusals(fx: &Fx, url: &str, at: i64) {
    let no_key = fork_body(at, json!({"provider": "openai", "model": "x"}));
    let (st, v) = fx.post(url, no_key).await;
    assert_eq!(
        (st, v["error"].as_str()),
        (400, Some("open mode needs OPENAI_API_KEY"))
    );
    let (st, _) = fx
        .post(url, fork_body(at, json!({"provider": "local:box"})))
        .await;
    assert_eq!(st, 400);
}

/// The child ran on `local:box` with model `m2`, from the fork point on.
fn assert_child_on_box(child: &Value, other: &FakeUpstream) {
    assert_ne!(child["status"], "infra_error", "{child}");
    let spec = &child["team"]["agents"][0];
    assert_eq!(
        (&spec["provider"], &spec["model"]),
        (&json!("local:box"), &json!("m2"))
    );
    let reqs = other.requests();
    assert_eq!((reqs.len(), reqs[0]["model"].as_str()), (1, Some("m2")));
    let calls = payloads(child, "model_call", "solo");
    assert_eq!(calls.last().unwrap()["provider"], "local:box");
}

#[test]
fn a_fork_moves_an_agent_to_another_provider() {
    run(async {
        let read = tool_msg("r1", "get_order_details", json!({"order_id": "#W7181492"}));
        let first = FakeUpstream::start(vec![read, text_msg("Done.")]).await;
        let other = FakeUpstream::start(vec![text_msg("Done again.")]).await;
        let mut providers = ProviderConfig::with_upstream(&first.base_url);
        providers.locals.push(local("box", &other.base_url, None));
        let fx = setup("fork", providers).await;
        let b = fx
            .run_to_end(&team(vec![agent("solo", None, Some("m1"))]))
            .await;
        let (id, at) = (b["id"].as_str().unwrap(), first_model_call(&b));
        let url = format!("/api/branches/{id}/fork");
        assert_fork_refusals(&fx, &url, at).await;
        let to_box = fork_body(at, json!({"provider": "local:box", "model": "m2"}));
        let (st, v) = fx.post(&url, to_box).await;
        assert_eq!(st, 200, "{v}");
        // Like a model change: the open agent resumes from the model call at the fork point.
        assert_eq!(
            (v["start_seq"].as_i64(), &v["moved_from"]),
            (Some(at), &Value::Null)
        );
        let child = fx.wait(v["branch_id"].as_str().unwrap()).await;
        assert_child_on_box(&child, &other);
        assert_eq!(first.requests().len(), 2, "the parent's calls only");
    })
}

#[test]
fn records_and_teams_without_a_provider_mean_openrouter() {
    let old = json!({"id": "a", "name": "A", "kind": "open", "model": null, "prompt": "p"});
    let spec: AgentSpec = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(spec.provider, None);
    assert_eq!(
        serde_json::to_value(&spec).unwrap(),
        old,
        "the shape is unchanged"
    );
    let p = ProviderConfig::default()
        .resolve(spec.provider.as_deref())
        .unwrap();
    assert_eq!(
        (p.id.as_str(), p.kind),
        ("openrouter", ProviderKind::OpenRouter)
    );
    let call = json!({"request": {}, "response": {}, "usage": {"cost": 0.1},
                      "cost_usd": 0.1, "cost_source": "openrouter"});
    let mc: ModelCallPayload = serde_json::from_value(call.clone()).unwrap();
    assert_eq!(
        (mc.provider.clone(), mc.provider_response.clone()),
        (None, None)
    );
    assert_eq!(serde_json::to_value(&mc).unwrap(), call);
    let change: Change = serde_json::from_value(json!({"model": "m"})).unwrap();
    assert_eq!(change.provider, None);
}
