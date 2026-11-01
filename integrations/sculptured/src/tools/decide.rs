//! `decide_change`: answers the question Sculptured asked about a change, as the person in Needs
//! you would (`POST /api/questions/{id}/answer`); Sculptured then records the decision. A change
//! whose question is gone (a restored server withdraws the questions its predecessor asked) can
//! no longer be decided.
use anyhow::Result;
use reqwest::Method;
use serde_json::{json, Value};

use super::{wrote, Outcome};
use crate::api::{Api, Reply};
use crate::changes;
use crate::poll;

/// The arguments of `decide_change`.
struct Decision {
    id: String,
    approved: bool,
    note: Option<String>,
}

impl Decision {
    fn parse(args: &Value) -> Result<Decision, String> {
        let id = match args["id"].as_str() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Err("id is required (text)".into()),
        };
        let approved = args["approved"]
            .as_bool()
            .ok_or("approved is required (true or false)")?;
        let note = match &args["note"] {
            Value::Null => None,
            Value::String(n) if !approved => Some(n.clone()),
            Value::String(_) => return Err("note goes only with approved: false".into()),
            _ => return Err("note must be text".into()),
        };
        Ok(Decision { id, approved, note })
    }

    /// The answer a person gives in Needs you.
    fn answer(&self) -> Value {
        match self.approved {
            true => json!({"kind": "allow"}),
            false => json!({"kind": "deny", "reason": self.note}),
        }
    }
}

pub async fn decide(api: &Api, team: &str, args: &Value) -> Result<Outcome> {
    let d = match Decision::parse(args) {
        Ok(d) => d,
        Err(e) => return Ok(Err(e)),
    };
    let all = changes::fetch(api, team).await?;
    let Some(change) = changes::find(&all, &d.id) else {
        return Ok(Err(format!("no change {} on team {team}", d.id)));
    };
    if let Some(was) = said(&change["decision"]) {
        return Ok(Err(format!("{} is already decided: {was}", d.id)));
    }
    let question = match question(api, team, &d.id).await? {
        Ok(q) => q,
        Err(e) => return Ok(Err(e)),
    };
    let answer = json!({"answer": d.answer()});
    let segments = ["questions", question.as_str(), "answer"];
    if let Reply::Refused { message, .. } = api.send(Method::POST, &segments, answer).await? {
        return Ok(Err(message));
    }
    let decided = poll::until("the decision recorded", || async {
        let all = changes::fetch(api, team).await?;
        let c = changes::find(&all, &d.id).filter(|c| !c["decision"].is_null());
        Ok(c.cloned())
    });
    Ok(Ok(wrote(decided.await?)))
}

/// A decision in words ("not approved by person"), or None while the change waits.
fn said(decision: &Value) -> Option<String> {
    let verdict = match decision["approved"].as_bool()? {
        true => "approved",
        false => "not approved",
    };
    Some(format!(
        "{verdict} by {}",
        decision["by"].as_str().unwrap_or("unknown")
    ))
}

/// The open question Sculptured asked about change `id`, or why it can no longer be decided.
async fn question(api: &Api, team: &str, id: &str) -> Result<Result<String, String>> {
    let all = api.get(&["teams", team, "questions"], &[]).await?;
    let all = all.expect("the team's questions")?;
    let about = |q: &&Value| q["prompt"]["tool_input"]["proposal_id"] == id;
    let asked: Vec<&Value> = all.as_array().into_iter().flatten().filter(about).collect();
    if let Some(q) = asked.iter().find(|q| q["state"] == "open") {
        return Ok(Ok(q["id"].as_str().unwrap_or_default().to_string()));
    }
    let why = asked
        .first()
        .and_then(|q| q["withdrawn_reason"].as_str().or(q["state"].as_str()))
        .unwrap_or("nobody was asked");
    Ok(Err(format!(
        "Sculptured can no longer decide {id} ({why}); propose it again"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_and_their_answers() {
        let yes = Decision::parse(&json!({"id": "chg-1", "approved": true})).unwrap();
        assert_eq!(yes.answer(), json!({"kind": "allow"}));
        let no = Decision::parse(&json!({"id": "chg-1", "approved": false, "note": "too dear"}));
        assert_eq!(
            no.unwrap().answer(),
            json!({"kind": "deny", "reason": "too dear"})
        );
        for bad in [
            json!({"approved": true}),
            json!({"id": "", "approved": true}),
            json!({"id": "chg-1"}),
            json!({"id": "chg-1", "approved": "yes"}),
            json!({"id": "chg-1", "approved": true, "note": "fine"}),
            json!({"id": "chg-1", "approved": false, "note": 3}),
        ] {
            assert!(Decision::parse(&bad).is_err(), "{bad}");
        }
    }
}
