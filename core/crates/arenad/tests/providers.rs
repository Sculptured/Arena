//! Model providers end to end: `/api/providers` and `/api/health` report them, a run or a fork
//! is refused on a provider that can't take it, and old records mean OpenRouter. `live_`: an
//! agent on a real provider (the one whose key is set; see `support/live.rs`) is recorded with
//! that provider's name and cost label, its model list is read with its key, and a key the
//! provider turns down is reported and never recorded. How each provider's request is shaped
//! (its key's header, the usage flag, Claude's Messages API both ways) is tested on the
//! adapters' own functions in `src/providers`, and retries on the wait they compute in
//! `src/agents/open.rs`.

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
    time::{Duration, Instant},
};

use arenad::{
    app::{AppState, Config},
    gateway::REJECTED_MESSAGE,
    model::*,
    providers::{
        Endpoint, LocalEndpoint, ProviderConfig, ProviderKind, Secret, ANTHROPIC_BASE,
        KEY_REJECTED, NOT_CHECKED, OPENAI_BASE,
    },
};
use live::Live;
use packages::TestHub;
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

fn agent(id: &str, provider: Option<&str>, model: Option<&str>) -> AgentSpec {
    AgentSpec {
        tools: Vec::new(),
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: model.map(str::to_string),
        prompt: format!("You are {id}."),
        provider: provider.map(str::to_string),
        read_only: false,
        lead: false,
        url: None,
        headers: Default::default(),
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
    let (state, base) = app_support::app(store, hub.hub.clone(), |c| c.providers = providers).await;
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

    /// Runs `team` on task 27 to its end; the branch as the API shows it.
    async fn run_to_end(&self, team: &Team) -> Value {
        let (st, v) = self.start(team).await;
        assert_eq!(st, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap();
        live::ended(&self.state, branch).await;
        self.get(&format!("/api/branches/{branch}")).await.1
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

/// A test server on a free port, stopped when dropped.
struct Server {
    base: String,
    task: tokio::task::AbortHandle,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A server that accepts connections and never answers.
async fn slow_server() -> Server {
    let (listener, base) = app_support::bind().await;
    let task = tokio::spawn(async move {
        let mut held = vec![];
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    Server {
        base,
        task: task.abort_handle(),
    }
}

/// The base of a port nothing listens on.
async fn down_server() -> String {
    let (listener, base) = app_support::bind().await;
    drop(listener);
    base
}

fn by_id<'a>(list: &'a Value, id: &str) -> &'a Value {
    let found = list.as_array().unwrap().iter().find(|p| p["id"] == id);
    found.unwrap_or_else(|| panic!("no {id} in {list}"))
}

/// No key, or a server that didn't answer: each unavailable, with the reason.
fn assert_unavailable(list: &Value) {
    let or = by_id(list, "openrouter");
    assert_eq!(
        (&or["available"], &or["why"], &or["checked"]),
        (
            &json!(false),
            &json!("OPENROUTER_API_KEY is not set"),
            &json!(false)
        )
    );
    let an = by_id(list, "anthropic");
    assert_eq!(
        (&an["available"], &an["why"]),
        (&json!(false), &json!("ANTHROPIC_API_KEY is not set"))
    );
    for id in ["local:slow1", "local:slow2"] {
        let s = by_id(list, id);
        assert_eq!(
            (&s["available"], &s["checked"]),
            (&json!(false), &json!(true)),
            "{s}"
        );
        assert!(s["why"].as_str().unwrap().contains("within 3 s"), "{s}");
    }
    let down = by_id(list, "local:down");
    assert_eq!(down["available"], false);
    assert!(
        down["why"].as_str().unwrap().contains("did not answer"),
        "{down}"
    );
}

/// The providers list asks every server that lists its models at once, each within its bound,
/// and says why one is unavailable; health answers from the configuration alone, at once.
#[test]
fn providers_route_reports_availability_within_the_bound() {
    run(async {
        let slow = [slow_server().await, slow_server().await];
        let providers = ProviderConfig {
            locals: vec![
                local("slow1", &slow[0].base, None),
                local("slow2", &slow[1].base, None),
                local("down", &down_server().await, None),
            ],
            ..ProviderConfig::default()
        };
        let fx = setup("list", providers).await;
        let t = Instant::now();
        let (st, list) = fx.get("/api/providers").await;
        let took = t.elapsed();
        assert_eq!(st, 200, "{list}");
        // Two servers that never answer, asked at once, each cut off at 3 s.
        assert!(took < Duration::from_millis(5500), "took {took:?}");
        assert_unavailable(&list);
        let t = Instant::now();
        let (_, h) = fx.get("/api/health").await;
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(h["open_available"], true, "a local server needs no key");
        // A local server it doesn't ask is not reported available.
        let summary = by_id(&h["providers"], "local:down");
        let seen = (&summary["available"], &summary["checked"], &summary["why"]);
        assert_eq!(seen, (&json!(false), &json!(false), &json!(NOT_CHECKED)));
        assert_eq!(summary.get("models"), None);
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
        "open mode needs ANTHROPIC_API_KEY",
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
        let providers = ProviderConfig {
            locals: vec![local("up", app_support::NOWHERE, None)],
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
        assert!(fx.state.store.runs().await.unwrap().is_empty());
    })
}

/// The fork of a branch at `at` that `changes` asks for.
fn fork_body(at: i64, changes: Value) -> Value {
    json!({"at_seq": at, "changes": {"solo": changes}})
}

/// A finished branch of `t`, written to the store by hand (no agent ran it): its id.
async fn finished_by_hand(fx: &Fx, t: &Team) -> String {
    let store = &fx.state.store;
    let run_id = store.create_run("27", t).await.unwrap();
    let b = store
        .create_branch(&run_id, None, t, &json!([]), None)
        .await
        .unwrap();
    store
        .set_status(&b, BranchStatus::Passed, None)
        .await
        .unwrap();
    b
}

/// A fork is refused before anything starts, judged on the team it would run: a provider
/// without its key, a new provider without a model, a model cleared on a provider that has no
/// default. The branch is recorded by hand (no agent ran it); the refusals need only its team.
#[test]
fn a_fork_is_refused_on_a_provider_that_cannot_take_it() {
    run(async {
        let mut providers = ProviderConfig::with_upstream(app_support::NOWHERE);
        providers
            .locals
            .push(local("box", app_support::NOWHERE, None));
        let fx = setup("forkrefuse", providers).await;
        let b = finished_by_hand(&fx, &team(vec![agent("solo", None, Some("m1"))])).await;
        let url = format!("/api/branches/{b}/fork");
        let no_default = |p: &str| format!("provider {p} has no default model: name one");
        let refused = [
            (
                json!({"provider": "openai", "model": "x"}),
                "open mode needs OPENAI_API_KEY".to_string(),
            ),
            (
                json!({"provider": "openrouter"}),
                "open mode needs OPENROUTER_API_KEY".to_string(),
            ),
            // A new provider drops the old model: the change must name one.
            (json!({"provider": "local:box"}), no_default("local:box")),
            // Clearing the model leaves the agent's own provider without one.
            (json!({"model": " "}), no_default("local:custom")),
        ];
        for (change, why) in refused {
            let (st, v) = fx.post(&url, fork_body(1, change.clone())).await;
            let seen = (st, v["error"].as_str());
            assert_eq!(seen, (400, Some(why.as_str())), "{change}");
        }
        let runs = fx.state.store.runs().await.unwrap();
        assert_eq!(runs[0].branches.len(), 1, "no branch was made");
    })
}

#[test]
fn records_and_teams_without_a_provider_mean_openrouter() {
    let old = json!({"id": "a", "name": "A", "kind": "open", "model": null, "prompt": "p"});
    let spec: AgentSpec = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(spec.provider, None);
    // No provider is written back; `read_only` always is (see `AgentSpec::read_only`).
    let mut stored = old.clone();
    stored["read_only"] = json!(false);
    assert_eq!(
        serde_json::to_value(&spec).unwrap(),
        stored,
        "no provider is added"
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

// ---------------------------------------------------------------- real providers

/// The cost and its label a successful call on `provider` records: OpenRouter's own, nothing
/// for OpenAI and Claude (no price table is kept), none charged on a local server.
fn assert_cost(provider: &str, mc: &ModelCallPayload) {
    match provider {
        "openrouter" => {
            assert!(mc.cost_usd.is_some(), "{:?}", mc.usage);
            assert_eq!(mc.cost_source.as_deref(), Some("openrouter"));
        }
        p if p.starts_with("local:") => {
            assert_eq!(
                (mc.cost_usd, mc.cost_source.as_deref()),
                (Some(0.0), Some("local (no API charge)"))
            );
        }
        _ => assert_eq!((mc.cost_usd, mc.cost_source.as_deref()), (None, None)),
    }
}

/// An agent on the real provider whose key is set: each model call names the provider and
/// carries its cost label; the canonical request is recorded (no provider-only fields), Claude's
/// own answer beside it; the agent's end sums them; and no key is in the record or in what
/// `Debug` prints of the configuration.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_an_agent_calls_its_own_provider_and_records_its_cost() {
    run_live(
        Live::api("live_an_agent_calls_its_own_provider_and_records_its_cost"),
        an_agent_calls_its_own_provider_and_records_its_cost,
    );
}

async fn an_agent_calls_its_own_provider_and_records_its_cost(live: Live) {
    let providers = Config::new().providers;
    let fx = setup("route", providers).await;
    let solo = live.agent("solo", "You are a retail support agent. Look up order #W7181492 with get_order_details, then answer in one sentence.");
    let b = fx.run_to_end(&team(vec![solo])).await;
    assert_ne!(b["status"], "infra_error", "{b}");
    let provider = live.provider().unwrap();
    let calls = payloads(&b, "model_call", "solo");
    assert!(!calls.is_empty());
    for c in &calls {
        let mc: ModelCallPayload = serde_json::from_value(c.clone()).unwrap();
        assert_eq!(mc.provider.as_deref(), Some(provider));
        let canonical = mc.request.get("usage").is_none();
        assert!(canonical, "the canonical request is recorded");
        assert_cost(provider, &mc);
        let claude = provider == "anthropic";
        assert_eq!(mc.provider_response.is_some(), claude, "{provider}");
    }
    let start = &payloads(&b, "agent_start", "solo")[0];
    assert_eq!(start["provider"], provider);
    let end = &payloads(&b, "agent_end", "solo")[0];
    if provider != "openrouter" {
        assert_eq!(end["cost_source"], calls[0]["cost_source"]);
    }
    assert_no_key_in(&b, &fx.state.config);
}

/// No configured key is in the record, nor in what `Debug` prints of the configuration.
fn assert_no_key_in(record: &Value, config: &Config) {
    let config = format!("{config:?}");
    for k in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
        if let Ok(key) = std::env::var(k) {
            assert!(!record.to_string().contains(&key), "{k} recorded");
            assert!(!config.contains(&key), "{k} in Debug");
        }
    }
}

/// The real provider's model list is read with its key: available, with its models.
#[test]
#[ignore = "live: calls a real provider (support/live.rs)"]
fn live_a_providers_models_are_listed_with_its_key() {
    run_live(
        Live::api("live_a_providers_models_are_listed_with_its_key"),
        a_providers_models_are_listed_with_its_key,
    );
}

async fn a_providers_models_are_listed_with_its_key(live: Live) {
    let fx = setup("models", Config::new().providers).await;
    let (st, list) = fx.get("/api/providers").await;
    assert_eq!(st, 200, "{list}");
    let p = by_id(&list, live.provider().unwrap());
    assert_eq!(
        (&p["available"], &p["checked"]),
        (&json!(true), &json!(true)),
        "{p}"
    );
    let models = p["models"].as_array().unwrap();
    assert!(!models.is_empty(), "{p}");
    let mut sorted = models.clone();
    sorted.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(models, &sorted);
}

const BAD_KEY: &str = "sk-arena-test-not-a-key";

/// OpenAI and the Claude API turn a key down (no charge: no model answers): the providers list
/// says so, and an agent's call on it is recorded with the answer's status but not its message,
/// which may quote the key.
#[test]
#[ignore = "live: calls real providers over the network (no key needed)"]
fn live_a_rejected_key_is_reported_and_its_message_not_recorded() {
    run(async {
        let providers = ProviderConfig {
            openai: Endpoint::new(OPENAI_BASE, Some(Secret::new(BAD_KEY))),
            anthropic: Endpoint::new(ANTHROPIC_BASE, Some(Secret::new(BAD_KEY))),
            ..ProviderConfig::default()
        };
        let fx = setup("reject", providers).await;
        let (_, list) = fx.get("/api/providers").await;
        for id in ["openai", "anthropic"] {
            let p = by_id(&list, id);
            assert_eq!(
                (&p["available"], &p["why"]),
                (&json!(false), &json!(KEY_REJECTED)),
                "{p}"
            );
        }
        for (provider, model) in [
            ("openai", "gpt-4.1-mini"),
            ("anthropic", "claude-haiku-4-5"),
        ] {
            let a = agent("solo", Some(provider), Some(model));
            let b = fx.run_to_end(&team(vec![a])).await;
            let call = &payloads(&b, "model_call", "solo")[0];
            assert_eq!(call["response"]["status"], 401, "{call}");
            let message = &call["response"]["body"]["error"]["message"];
            assert_eq!(message, REJECTED_MESSAGE, "{call}");
            assert!(!b.to_string().contains(BAD_KEY), "the key is recorded");
            assert_eq!(b["status"], "infra_error", "{provider}");
        }
    })
}
