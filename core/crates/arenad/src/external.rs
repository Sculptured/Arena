//! Runs an external system drives: the design's own code (or Sculptured) registers its agents,
//! gets each one's MCP and gateway endpoints, drives them however it likes, and says when the run
//! is finished. arenad records everything they do through those endpoints, as for its own runners.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{exec::BranchCtx, model::*, store::Store};

/// Where an agent reaches the world's tools (MCP, streamable HTTP) and its models (an
/// OpenAI-compatible base URL: `{gateway_url}/chat/completions`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Endpoints {
    pub agent: String,
    pub mcp_url: String,
    pub gateway_url: String,
}

/// The endpoints of `agent` on `branch`, on the arenad at `self_base`.
pub fn endpoints(self_base: &str, branch: &str, agent: &str) -> Endpoints {
    let base = self_base.trim_end_matches('/');
    Endpoints {
        agent: agent.to_string(),
        mcp_url: format!("{base}/mcp/{branch}/{agent}"),
        gateway_url: format!("{base}/gateway/{branch}/{agent}/v1"),
    }
}

/// An agent an external system registers. Only `id` is needed; `kind` defaults to `external`.
#[derive(Debug, Clone, Deserialize)]
pub struct Join {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<AgentKind>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub read_only: bool,
}

impl Join {
    pub fn spec(&self) -> AgentSpec {
        AgentSpec {
            id: self.id.clone(),
            name: self.name.clone().unwrap_or_else(|| self.id.clone()),
            kind: self.kind.unwrap_or(AgentKind::External),
            model: self.model.as_deref().and_then(trimmed),
            prompt: self.prompt.clone(),
            provider: self.provider.as_deref().and_then(trimmed),
            read_only: self.read_only,
        }
    }
}

/// The `AgentStart` of an agent an external system (re)started.
pub fn start_payload(spec: &AgentSpec) -> Value {
    let mut start = json!({"name": spec.name, "kind": spec.kind, "model": spec.model,
                           "external": true});
    if let Some(p) = &spec.provider {
        start["provider"] = json!(p);
    }
    start
}

/// Registers `join` on the live external `ctx` and records its `AgentStart`. An agent the branch
/// knows already (one of its team, or one that joined before) is started again instead. The
/// branch's stored team gets the new agent.
pub async fn register(store: &Store, ctx: &BranchCtx, join: &Join) -> anyhow::Result<AgentSpec> {
    let spec = match ctx.agent(&join.id) {
        Some(known) => known,
        None => {
            let spec = join.spec();
            if ctx.join(spec.clone()) {
                store.set_team(&ctx.branch_id, &team_with(ctx)).await?;
            }
            spec
        }
    };
    let start = start_payload(&spec);
    ctx.append(store, Some(&spec.id), EventKind::AgentStart, start)
        .await?;
    Ok(spec)
}

/// The branch's team with the agents that joined it.
fn team_with(ctx: &BranchCtx) -> Team {
    Team {
        agents: ctx.agents(),
        ..ctx.team.clone()
    }
}

/// What an external system reports when one of its agents is done.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentDone {
    #[serde(default = "yes")]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    /// The cost the system's provider reported, with where it comes from.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub cost_source: Option<String>,
    #[serde(default)]
    pub seconds: Option<f64>,
}

fn yes() -> bool {
    true
}

impl AgentDone {
    pub fn payload(&self) -> AgentEndPayload {
        let total = self.prompt_tokens.zip(self.completion_tokens);
        AgentEndPayload {
            ok: self.ok,
            error: self.error.clone(),
            tokens: total.map(|(p, c)| p + c),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cost_usd: self.cost_usd,
            cost_source: self.cost_source.clone().filter(|_| self.cost_usd.is_some()),
            seconds: self.seconds,
        }
    }
}

/// One model call a CLI agent's client saw, as its stream reports it (Claude Code's per-message
/// `usage`), or a compaction of its context. arenad never sees a CLI's model calls itself, so the
/// design reports them and arenad records each as a `ModelCall` without request or response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UsageReport {
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    /// The model's context window, when the client says.
    #[serde(default)]
    pub context_window: Option<i64>,
    /// Set when the client compacted the agent's context instead: `{trigger, pre_tokens}`.
    #[serde(default)]
    pub compaction: Option<Value>,
}

/// Where a reported model call comes from, in its payload's `reported_by`.
pub const REPORTED_BY_CLIENT: &str = "client";

impl UsageReport {
    /// The call's usage: its whole input (fresh, cache-written and cache-read tokens) is its
    /// `prompt_tokens`, the context it was sent with. A compaction has none.
    fn usage(&self) -> Value {
        if self.compaction.is_some() {
            return json!({});
        }
        let prompt =
            self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens;
        json!({"prompt_tokens": prompt, "completion_tokens": self.output_tokens,
               "total_tokens": prompt + self.output_tokens,
               "input_tokens": self.input_tokens,
               "cache_creation_input_tokens": self.cache_creation_input_tokens,
               "cache_read_input_tokens": self.cache_read_input_tokens})
    }

    /// The `ModelCall` payload, with no request or response.
    pub fn payload(&self) -> Value {
        let usage = self.usage();
        let call = ModelCallPayload {
            request: Value::Null,
            response: Value::Null,
            usage,
            cost_usd: None,
            cost_source: None,
            provider: None,
            provider_response: None,
        };
        let mut v = serde_json::to_value(call).expect("a payload serializes");
        v["reported_by"] = json!(REPORTED_BY_CLIENT);
        let extra = [
            ("message_id", json!(self.message_id)),
            ("model", json!(self.model)),
            ("context_window", json!(self.context_window)),
            ("compaction", json!(self.compaction)),
        ];
        for (k, x) in extra.into_iter().filter(|(_, x)| !x.is_null()) {
            v[k] = x;
        }
        v
    }
}

/// One agent's recorded history before `before_seq` (its steps, the notes and messages it sent
/// and got), with its endpoints on the branch that resumes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resumable {
    #[serde(flatten)]
    pub endpoints: Endpoints,
    pub spec: AgentSpec,
    pub history: Vec<Event>,
}

/// True when `e` belongs to `agent`'s history: its own step, or a note or message for it.
fn in_history(e: &Event, agent: &str) -> bool {
    let to = e.payload.get("to").and_then(Value::as_str);
    let sent_to_it = matches!(e.kind, EventKind::Handoff | EventKind::Message)
        && to.is_some_and(|t| t == agent || t == EVERYONE);
    e.agent.as_deref() == Some(agent) || sent_to_it
}

/// Each agent of `team` with its history in `events` before `before_seq`, for resuming on
/// `branch` at `self_base`.
pub fn resumables(
    self_base: &str,
    branch: &str,
    team: &Team,
    events: &[Event],
    before_seq: i64,
) -> Vec<Resumable> {
    let before: Vec<&Event> = events.iter().filter(|e| e.seq < before_seq).collect();
    team.agents
        .iter()
        .map(|spec| Resumable {
            endpoints: endpoints(self_base, branch, &spec.id),
            spec: spec.clone(),
            history: before
                .iter()
                .filter(|e| in_history(e, &spec.id))
                .map(|e| (*e).clone())
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: i64, agent: Option<&str>, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: chrono::Utc::now(),
            agent: agent.map(Into::into),
            kind,
            payload,
        }
    }

    #[test]
    fn a_resumed_agent_gets_its_own_history_up_to_the_fork_point() {
        let spec = |id: &str| {
            Join {
                id: id.into(),
                ..join()
            }
            .spec()
        };
        let (id, name) = ("t".into(), "T".into());
        let team = Team {
            id,
            name,
            agents: vec![spec("a"), spec("b")],
        };
        let evs = vec![
            ev(1, None, EventKind::RunStart, json!({})),
            ev(2, Some("a"), EventKind::AgentStart, json!({})),
            ev(
                3,
                Some("a"),
                EventKind::Message,
                json!({"to": "b", "text": "hi"}),
            ),
            ev(4, Some("b"), EventKind::ToolCall, json!({"name": "get"})),
            ev(5, Some("a"), EventKind::ToolCall, json!({"name": "late"})),
        ];
        let r = resumables("http://h:1/", "br", &team, &evs, 5);
        let seqs = |i: usize| r[i].history.iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!((seqs(0), seqs(1)), (vec![2, 3], vec![3, 4]));
        assert_eq!(r[1].endpoints.mcp_url, "http://h:1/mcp/br/b");
        assert_eq!(r[1].endpoints.gateway_url, "http://h:1/gateway/br/b/v1");
    }

    #[test]
    fn a_reported_call_counts_the_cache_in_its_context() {
        let r: UsageReport = serde_json::from_value(json!({"message_id": "m1", "model": "x",
            "input_tokens": 2, "cache_creation_input_tokens": 100,
            "cache_read_input_tokens": 900, "output_tokens": 7, "context_window": 200000}))
        .unwrap();
        let p = r.payload();
        assert_eq!(p["usage"]["prompt_tokens"], 1002);
        assert_eq!(p["usage"]["total_tokens"], 1009);
        assert_eq!(
            (&p["reported_by"], &p["context_window"]),
            (&json!("client"), &json!(200000))
        );
        assert!(p["request"].is_null() && p.get("compaction").is_none());
        let c: UsageReport =
            serde_json::from_value(json!({"compaction": {"trigger": "auto", "pre_tokens": 9}}))
                .unwrap();
        let p = c.payload();
        assert_eq!(
            (&p["usage"], &p["compaction"]["pre_tokens"]),
            (&json!({}), &json!(9))
        );
    }

    fn join() -> Join {
        serde_json::from_value(json!({"id": "x"})).unwrap()
    }

    #[test]
    fn a_registered_agent_is_external_unless_it_says_otherwise() {
        let s = join().spec();
        assert_eq!(
            (s.kind, s.name.as_str(), s.read_only),
            (AgentKind::External, "x", false)
        );
        let done: AgentDone =
            serde_json::from_value(json!({"prompt_tokens": 3, "completion_tokens": 2})).unwrap();
        let p = done.payload();
        assert_eq!((p.ok, p.tokens, p.cost_source), (true, Some(5), None));
    }
}
