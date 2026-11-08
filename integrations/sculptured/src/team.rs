//! The team whose changes the tools propose and decide: a plain Sculptured team with no agents.
//! The tools propose its changes as an outside system (`POST /api/teams/{team}/changes`, proposer
//! `arena`), so the team has no rules and every change waits for a person's decision
//! (`decide_change`), for up to a day.
use anyhow::{bail, Result};
use reqwest::Method;
use serde_json::json;

use crate::api::Api;

/// How many changes may wait for a decision at once.
pub const MAX_WAITING: usize = 8;
/// How long a change waits for a decision before Sculptured records nobody answered in time
/// (its longest question timeout: a day).
const DECISION_WAIT_MIN: u32 = 1440;

/// Makes `team` exist: made fresh, with the longest decision wait, when it doesn't.
pub async fn ensure(api: &Api, team: &str) -> Result<()> {
    if !exists(api, team).await? {
        create(api, team).await?;
    }
    Ok(())
}

async fn exists(api: &Api, team: &str) -> Result<bool> {
    use crate::api::Reply;
    match api.get(&["teams", team], &[]).await? {
        Reply::Ok(_) => Ok(true),
        Reply::Refused { status: 404, .. } => Ok(false),
        r => r.expect("reading the team").map(|_| true),
    }
}

/// A new team named `team` with no agents and the longest decision wait.
async fn create(api: &Api, team: &str) -> Result<()> {
    let new = json!({"template": "blank", "name": team});
    let made = api.send(Method::POST, &["teams"], new).await?;
    let draft = made.expect("making the team")?["draft"].take();
    if draft["id"] != team {
        bail!("Sculptured named team {team:?} {}", draft["id"]);
    }
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
