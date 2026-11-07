//! The team whose changes the tools propose and decide. Changes come from Practice agents (no
//! model): each proposes one change at a time through Sculptured's own `propose_change` tool,
//! and its call waits there for the decision, so the team has `PROPOSERS` of them. The team has
//! no rules, so every change waits for a person's decision (`decide_change`), for up to a day.
use anyhow::{bail, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::Api;
use crate::poll;

/// How many changes may wait for a decision at once: one per Practice agent.
pub const PROPOSERS: usize = 8;
/// How long a change waits for a decision before Sculptured records nobody answered in time
/// (its longest question timeout: a day).
const DECISION_WAIT_MIN: u32 = 1440;

/// An agent of the team as `GET /api/rigs/{team}` shows it.
#[derive(Debug, Clone, Deserialize)]
pub struct Seat {
    pub id: String,
    pub lifecycle: String,
    #[serde(default)]
    pub open_questions: u64,
}

impl Seat {
    pub fn running(&self) -> bool {
        self.lifecycle == "running"
    }
}

/// Makes `team` exist (made fresh when it doesn't) and (re)starts its agents: a restored server
/// lost the agents' processes. Returns once every agent runs.
pub async fn ensure_running(api: &Api, team: &str) -> Result<()> {
    if exists(api, team).await? {
        // A restored runner can resume the saved team before we reach this call.
        // Stop that generation before starting fresh proposers for re-raised changes.
        api.send(Method::POST, &["teams", team, "stop"], json!({}))
            .await?
            .expect("stopping the restored team")?;
    } else {
        create(api, team).await?;
    }
    let start = ["teams", team, "start"];
    let started = api.send(Method::POST, &start, json!({})).await?;
    started.expect("starting the team")?;
    let what = format!("every agent of team {team} running");
    poll::until(&what, || async {
        let s = seats(api, team).await?;
        Ok((s.len() == PROPOSERS && s.iter().all(Seat::running)).then_some(()))
    })
    .await
}

async fn exists(api: &Api, team: &str) -> Result<bool> {
    use crate::api::Reply;
    match api.get(&["teams", team], &[]).await? {
        Reply::Ok(_) => Ok(true),
        Reply::Refused { status: 404, .. } => Ok(false),
        r => r.expect("reading the team").map(|_| true),
    }
}

/// A new Practice team named `team` with the proposers and the longest decision wait.
async fn create(api: &Api, team: &str) -> Result<()> {
    let new = json!({"template": "blank", "practice": true, "name": team});
    let made = api.send(Method::POST, &["teams"], new).await?;
    let mut draft = made.expect("making the team")?["draft"].take();
    if draft["id"] != team {
        bail!("Sculptured named team {team:?} {}", draft["id"]);
    }
    draft["agents"] = proposers();
    let put = json!({"revision": draft["revision"], "draft": draft});
    let saved = api.send(Method::PUT, &["teams", team], put).await?;
    saved.expect("adding the team's agents")?;
    let segments = ["teams", team, "settings"];
    let mut settings = api
        .get(&segments, &[])
        .await?
        .expect("the team's settings")?;
    settings["question_timeout_min"] = json!(DECISION_WAIT_MIN);
    let put = json!({"settings": settings});
    api.send(Method::PUT, &segments, put)
        .await?
        .expect("the team's settings")?;
    Ok(())
}

fn proposers() -> Value {
    let agent = |n: usize| {
        json!({"id": format!("p{n}"), "group_id": null, "name": format!("Proposer {n}"),
               "kind": "practice", "job": "", "folder": null, "x": 40, "y": 100 * n})
    };
    Value::Array((1..=PROPOSERS).map(agent).collect())
}

/// The team's agents.
pub async fn seats(api: &Api, team: &str) -> Result<Vec<Seat>> {
    let rig = api
        .get(&["rigs", team], &[])
        .await?
        .expect("the team's agents")?;
    Ok(serde_json::from_value(rig["seats"].clone())?)
}
