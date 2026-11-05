//! A branch's numbers for deployment estimates: the wall time measured, arena's own overhead in
//! it, the time estimated in production (measured minus overhead), and tokens and cost per agent
//! and model. Only the branch's own events count (`seq >= start_seq`), as in compare: the
//! inherited prefix belongs to its parent.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::{
    model::*,
    overhead::Overhead,
    prices::{self, Prices},
    providers::LOCAL_COST_SOURCE,
};

pub const REPORTED: &str = "reported";
pub const ESTIMATE: &str = "list-price estimate";
pub const NO_CHARGE: &str = "no API charge";
pub const UNKNOWN: &str = "unknown";
pub const MIXED: &str = "mixed";

/// A cost and where it comes from: what the provider reported, a list-price estimate (with the
/// price's source and date), no API charge (a local model), or unknown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cost {
    pub usd: Option<f64>,
    pub label: String,
    /// The provider's cost source, or the URL a list price was read at.
    pub source: Option<String>,
    /// The date of a list price.
    pub as_of: Option<String>,
}

impl Cost {
    fn unknown() -> Cost {
        Cost::labelled(None, UNKNOWN)
    }

    fn labelled(usd: Option<f64>, label: &str) -> Cost {
        Cost {
            usd,
            label: label.into(),
            source: None,
            as_of: None,
        }
    }
}

/// What one agent used while it ran with one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentUse {
    pub agent: String,
    pub kind: Option<AgentKind>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub tokens: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cost: Cost,
    /// The model calls recorded for it (through the gateway, or reported by its client).
    pub model_calls: i64,
    /// The largest `prompt_tokens` of those calls: the most context it was sent with.
    pub peak_prompt_tokens: Option<i64>,
    /// How many times its client compacted its context (reported).
    pub compactions: i64,
    /// Arena's overhead recorded on this agent's own events, in seconds.
    pub overhead_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metrics {
    /// Wall time from the branch's creation to its end (or to now while it runs).
    pub measured_s: f64,
    /// Arena's own time in it, per kind, in milliseconds.
    pub overhead_ms: Overhead,
    pub overhead_s: f64,
    /// False for runs recorded before overhead was measured: then there is no estimate.
    pub overhead_recorded: bool,
    /// `measured_s - overhead_s` (never below 0), when overhead was recorded.
    pub estimated_s: Option<f64>,
    pub tokens: i64,
    pub cost: Cost,
    pub agents: Vec<AgentUse>,
}

/// Tokens and costs seen for one agent run, before its cost is settled.
#[derive(Default)]
struct Tally {
    ended: bool,
    tokens: i64,
    prompt: Option<i64>,
    completion: Option<i64>,
    reported: Option<f64>,
    source: Option<String>,
    calls: CallTally,
    overhead_ms: f64,
}

/// What the agent's model calls (through the gateway) add up to.
#[derive(Default)]
struct CallTally {
    prompt: i64,
    completion: i64,
    total: i64,
    cost: Option<f64>,
    all_costed: bool,
    any: bool,
    source: Option<String>,
    count: i64,
    peak: Option<i64>,
    compactions: i64,
}

fn opt_add(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    Some(a.unwrap_or(0) + b?)
}

impl CallTally {
    /// A compaction a client reported: no tokens of its own.
    fn compacted(&mut self) {
        self.compactions += 1;
    }

    fn add(&mut self, p: &ModelCallPayload) {
        let n = |k: &str| p.usage.get(k).and_then(Value::as_i64).unwrap_or(0);
        self.count += 1;
        if let Some(prompt) = p.usage.get("prompt_tokens").and_then(Value::as_i64) {
            self.peak = Some(self.peak.map_or(prompt, |m| m.max(prompt)));
        }
        if !self.any {
            self.all_costed = true;
        }
        self.any = true;
        self.prompt += n("prompt_tokens");
        self.completion += n("completion_tokens");
        self.total += n("total_tokens").max(n("prompt_tokens") + n("completion_tokens"));
        self.all_costed &= p.cost_usd.is_some();
        self.cost = add_cost(Some(self.cost.unwrap_or(0.0)), p.cost_usd);
        self.source = self.source.take().or_else(|| p.cost_source.clone());
    }
}

impl Tally {
    fn end(&mut self, e: &AgentEndPayload) {
        self.ended = true;
        self.tokens += e.tokens.unwrap_or(0);
        self.prompt = opt_add(self.prompt, e.prompt_tokens);
        self.completion = opt_add(self.completion, e.completion_tokens);
        if let Some(c) = e.cost_usd {
            self.reported = Some(self.reported.unwrap_or(0.0) + c);
        }
        self.source = self.source.take().or_else(|| e.cost_source.clone());
    }

    /// Tokens of the run: its ends', or its model calls' when it never ended (or kept no split).
    fn settle(&mut self) {
        let calls = &self.calls;
        if !self.ended && calls.any {
            self.tokens = calls.total;
            if calls.all_costed {
                self.reported = calls.cost;
                self.source = calls.source.clone();
            }
        }
        if self.prompt.is_none() && calls.any {
            (self.prompt, self.completion) = (Some(calls.prompt), Some(calls.completion));
        }
    }
}

/// The cost of one agent run: reported, else estimated from a list price, else unknown.
fn cost_of(t: &Tally, u: &AgentUse, table: &Prices) -> Cost {
    if let Some(usd) = t.reported {
        let local = t.source.as_deref() == Some(LOCAL_COST_SOURCE);
        let mut c = Cost::labelled(Some(usd), if local { NO_CHARGE } else { REPORTED });
        c.source = t.source.clone();
        return c;
    }
    let price = u
        .kind
        .and_then(|k| prices::provider_of(k, u.provider.as_deref()));
    let price = price.zip(u.model.as_deref());
    let price = price.and_then(|(p, m)| table.find(&p, m));
    match (price, t.prompt, t.completion) {
        (Some(p), Some(i), Some(o)) if t.tokens > 0 || i + o > 0 => Cost {
            usd: Some(p.cost(i, o)),
            label: ESTIMATE.into(),
            source: Some(p.source.clone()),
            as_of: Some(p.as_of.clone()),
        },
        _ => Cost::unknown(),
    }
}

/// One agent run, from its AgentStart.
fn agent_use(e: &Event) -> AgentUse {
    let p = &e.payload;
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    AgentUse {
        agent: e.agent.clone().unwrap_or_default(),
        kind: serde_json::from_value(p["kind"].clone()).ok(),
        model: text("model"),
        provider: text("provider"),
        tokens: 0,
        prompt_tokens: None,
        completion_tokens: None,
        cost: Cost::unknown(),
        model_calls: 0,
        peak_prompt_tokens: None,
        compactions: 0,
        overhead_s: 0.0,
    }
}

fn parsed<P: serde::de::DeserializeOwned>(e: &Event) -> Option<P> {
    serde_json::from_value(e.payload.clone()).ok()
}

/// The agent runs of `events`, each with what it used.
fn agent_uses(events: &[&Event], table: &Prices) -> Vec<AgentUse> {
    let mut runs: Vec<(AgentUse, Tally)> = vec![];
    for e in events {
        if e.kind == EventKind::AgentStart {
            runs.push((agent_use(e), Tally::default()));
            continue;
        }
        let agent = e.agent.as_deref();
        let run = runs
            .iter_mut()
            .rev()
            .find(|(u, _)| Some(u.agent.as_str()) == agent);
        let Some((_, t)) = run else {
            continue;
        };
        t.overhead_ms += event_overhead(e);
        match e.kind {
            EventKind::AgentEnd => parsed(e).iter().for_each(|p| t.end(p)),
            EventKind::ModelCall if e.payload.get("compaction").is_some() => t.calls.compacted(),
            EventKind::ModelCall => parsed(e).iter().for_each(|p| t.calls.add(p)),
            _ => {}
        }
    }
    runs.into_iter().map(|(u, t)| finish(u, t, table)).collect()
}

fn finish(mut u: AgentUse, mut t: Tally, table: &Prices) -> AgentUse {
    t.settle();
    u.cost = cost_of(&t, &u, table);
    (u.tokens, u.prompt_tokens, u.completion_tokens) = (t.tokens, t.prompt, t.completion);
    (u.model_calls, u.peak_prompt_tokens) = (t.calls.count, t.calls.peak);
    (u.compactions, u.overhead_s) = (t.calls.compactions, t.overhead_ms / 1000.0);
    u
}

/// The overhead recorded on one event, in milliseconds.
fn event_overhead(e: &Event) -> f64 {
    let o = e.payload.get("overhead_ms").cloned();
    let o = o.and_then(|o| serde_json::from_value::<Overhead>(o).ok());
    o.map_or(0.0, |o| o.total())
}

/// The branch's cost: the sum when every agent's is known, labelled with their one label (or
/// `mixed`); unknown as soon as one is.
fn total_cost(agents: &[AgentUse]) -> Cost {
    let mut labels: Vec<&str> = agents.iter().map(|a| a.cost.label.as_str()).collect();
    labels.dedup();
    let usd = agents
        .iter()
        .try_fold(0.0, |s, a| a.cost.usd.map(|c| s + c));
    match (usd, labels.as_slice()) {
        (None, _) | (_, []) => Cost::unknown(),
        (Some(usd), [one]) => {
            let first = &agents[0].cost;
            let same = agents.iter().all(|a| a.cost.source == first.source);
            Cost {
                source: first.source.clone().filter(|_| same),
                as_of: first.as_of.clone().filter(|_| same),
                ..Cost::labelled(Some(usd), one)
            }
        }
        (Some(usd), _) => Cost::labelled(Some(usd), MIXED),
    }
}

/// The overhead recorded on `events`, and whether any was.
fn overhead_of(events: &[&Event]) -> (Overhead, bool) {
    let mut sum = Overhead::default();
    let mut any = false;
    for e in events {
        let Some(o) = e.payload.get("overhead_ms") else {
            continue;
        };
        if let Ok(o) = serde_json::from_value::<Overhead>(o.clone()) {
            sum.merge(&o);
            any = true;
        }
    }
    (sum, any)
}

/// The metrics of `branch` from its `events`, at `now` while it runs, priced with `table`.
pub fn metrics(branch: &Branch, events: &[Event], table: &Prices, now: DateTime<Utc>) -> Metrics {
    let from = branch.start_seq.unwrap_or(0);
    let own: Vec<&Event> = events.iter().filter(|e| e.seq >= from).collect();
    let end = branch.ended_at.unwrap_or(now);
    let measured_s = ((end - branch.created_at).num_milliseconds() as f64 / 1000.0).max(0.0);
    let (overhead_ms, overhead_recorded) = overhead_of(&own);
    let overhead_s = overhead_ms.total() / 1000.0;
    let agents = agent_uses(&own, table);
    Metrics {
        measured_s,
        overhead_s,
        overhead_recorded,
        estimated_s: overhead_recorded.then(|| (measured_s - overhead_s).max(0.0)),
        overhead_ms,
        tokens: agents.iter().map(|a| a.tokens).sum(),
        cost: total_cost(&agents),
        agents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, agent: Option<&str>, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: Utc::now(),
            agent: agent.map(Into::into),
            kind,
            payload,
        }
    }

    fn branch(start_seq: Option<i64>, secs: i64) -> Branch {
        let created_at = Utc::now();
        Branch {
            id: "b".into(),
            run_id: "r".into(),
            parent_id: None,
            start_seq,
            team: Team {
                id: "t".into(),
                name: "T".into(),
                agents: vec![],
            },
            changes: json!([]),
            fork_note: None,
            status: BranchStatus::Passed,
            error: None,
            created_at,
            ended_at: Some(created_at + chrono::Duration::seconds(secs)),
            restore_mode: None,
            restored: vec![],
        }
    }

    fn table() -> Prices {
        Prices::parse(
            r#"[[price]]
provider = "anthropic"
model = "m"
input_per_mtok = 2.0
output_per_mtok = 10.0
source = "https://example.org/prices"
as_of = "2026-11-04""#,
        )
        .unwrap()
    }

    fn start(seq: i64, agent: &str, kind: &str, model: &str, provider: Option<&str>) -> Event {
        let mut p = json!({"name": agent, "kind": kind, "model": model});
        if let Some(pr) = provider {
            p["provider"] = json!(pr);
        }
        ev(seq, Some(agent), EventKind::AgentStart, p)
    }

    fn end(seq: i64, agent: &str, split: Option<(i64, i64)>, cost: Option<(f64, &str)>) -> Event {
        let (i, o) = split.unwrap_or((60, 40));
        let mut p = json!({"ok": true, "error": null, "tokens": i + o, "seconds": 1.0,
                           "cost_usd": cost.map(|c| c.0), "cost_source": cost.map(|c| c.1)});
        if split.is_some() {
            (p["prompt_tokens"], p["completion_tokens"]) = (json!(i), json!(o));
        }
        ev(seq, Some(agent), EventKind::AgentEnd, p)
    }

    #[test]
    fn the_estimate_is_measured_time_minus_overhead() {
        let evs = vec![
            ev(
                1,
                None,
                EventKind::RunStart,
                json!({"overhead_ms": {"restore": 1500.0}}),
            ),
            start(2, "a", "open", "m", Some("anthropic")),
            ev(
                3,
                Some("a"),
                EventKind::ToolCall,
                json!({"name": "t", "overhead_ms": {"save": 400.0, "record": 100.0}}),
            ),
            end(4, "a", Some((1_000_000, 100_000)), None),
        ];
        let m = metrics(&branch(None, 10), &evs, &table(), Utc::now());
        assert_eq!((m.measured_s, m.overhead_s), (10.0, 2.0));
        assert_eq!(m.estimated_s, Some(8.0));
        assert_eq!((m.overhead_ms.restore, m.overhead_ms.save), (1500.0, 400.0));
        assert_eq!(m.agents.len(), 1);
        assert_eq!(m.agents[0].cost.usd, Some(3.0));
        assert_eq!(m.cost.label, ESTIMATE);
        assert_eq!(m.cost.as_of.as_deref(), Some("2026-11-04"));
        assert_eq!(m.cost.source.as_deref(), Some("https://example.org/prices"));
    }

    #[test]
    fn old_runs_have_no_estimate_and_forks_count_only_their_own_steps() {
        let evs = vec![
            start(1, "a", "open", "m", Some("anthropic")),
            end(2, "a", None, Some((0.5, "openrouter"))),
            start(3, "b", "claude_code", "sonnet", None),
            end(4, "b", None, Some((0.25, "api_equivalent_estimate"))),
        ];
        let m = metrics(&branch(None, 4), &evs, &table(), Utc::now());
        assert!(!m.overhead_recorded && m.estimated_s.is_none());
        assert_eq!(
            (m.tokens, m.cost.usd, m.cost.label.as_str()),
            (200, Some(0.75), REPORTED)
        );
        let fork = metrics(&branch(Some(3), 4), &evs, &table(), Utc::now());
        assert_eq!((fork.tokens, fork.agents.len()), (100, 1));
    }

    #[test]
    fn a_cost_without_a_price_is_unknown_and_local_models_cost_nothing() {
        let evs = vec![
            start(1, "a", "open", "not-listed", Some("openai")),
            end(2, "a", Some((10, 5)), None),
            start(3, "b", "open", "llama", Some("local:box")),
            end(4, "b", Some((10, 5)), Some((0.0, LOCAL_COST_SOURCE))),
        ];
        let m = metrics(&branch(None, 4), &evs, &table(), Utc::now());
        assert_eq!(m.agents[0].cost.label, UNKNOWN);
        assert_eq!(m.agents[1].cost.label, NO_CHARGE);
        assert_eq!((m.cost.usd, m.cost.label.as_str()), (None, UNKNOWN));
        assert_eq!(m.tokens, 30);
    }

    /// A model call `agent`'s client reported: `prompt` tokens of context, or a compaction.
    fn reported(seq: i64, prompt: Option<i64>) -> Event {
        let mut p = json!({"request": null, "response": null, "cost_usd": null,
                           "cost_source": null, "usage": {}, "overhead_ms": {"mcp": 250.0}});
        match prompt {
            Some(n) => p["usage"] = json!({"prompt_tokens": n, "completion_tokens": 1}),
            None => p["compaction"] = json!({"pre_tokens": 190000}),
        }
        ev(seq, Some("c"), EventKind::ModelCall, p)
    }

    #[test]
    fn reported_calls_give_each_agent_its_peak_context_compactions_and_overhead() {
        let call = |seq, n| reported(seq, Some(n));
        let compaction = reported(4, None);
        let evs = vec![
            start(1, "c", "external", "m", None),
            call(2, 1200),
            call(3, 150_000),
            compaction,
            call(5, 40_000),
            end(6, "c", Some((191_200, 3)), None),
        ];
        let m = metrics(&branch(None, 9), &evs, &table(), Utc::now());
        let a = &m.agents[0];
        assert_eq!(
            (a.model_calls, a.peak_prompt_tokens, a.compactions),
            (3, Some(150_000), 1)
        );
        assert!((a.overhead_s - 1.0).abs() < 1e-9, "{}", a.overhead_s);
        assert_eq!(a.tokens, 191_203);
    }

    #[test]
    fn an_agent_that_never_ended_counts_its_model_calls() {
        let call = |seq, cost: Option<f64>| {
            ev(
                seq,
                Some("x"),
                EventKind::ModelCall,
                json!({"request": {}, "response": {},
               "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10},
               "cost_usd": cost, "cost_source": cost.map(|_| "openrouter")}),
            )
        };
        let evs = vec![
            start(1, "x", "open", "q", None),
            call(2, Some(0.1)),
            call(3, Some(0.2)),
        ];
        let m = metrics(&branch(None, 1), &evs, &table(), Utc::now());
        let a = &m.agents[0];
        assert_eq!(
            (a.tokens, a.prompt_tokens, a.completion_tokens),
            (20, Some(14), Some(6))
        );
        assert!((a.cost.usd.unwrap() - 0.3).abs() < 1e-9);
        let evs = vec![
            start(1, "x", "open", "q", None),
            call(2, Some(0.1)),
            call(3, None),
        ];
        let m = metrics(&branch(None, 1), &evs, &table(), Utc::now());
        assert_eq!(m.agents[0].cost.label, UNKNOWN);
    }
}
