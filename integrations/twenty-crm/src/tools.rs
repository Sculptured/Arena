//! The agents' tools, on Twenty's REST API. A refusal or a bad argument is the tool's error; only
//! Twenty not answering fails the call.
use anyhow::Error;
use arena_part::proto::ToolOutput;
use arena_part::PartError;
use serde_json::{json, Value};

use crate::rest::{Reply, Rest};

/// As many records as one answer lists; more than a test world holds.
const PAGE: &str = "60";

enum Failure {
    Refused(String),
    Down(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Failure {
        Failure::Down(e)
    }
}

type Out = Result<ToolOutput, Failure>;

fn given<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args[name].as_str().filter(|s| !s.trim().is_empty())
}

fn required<'a>(args: &'a Value, name: &str) -> Result<&'a str, Failure> {
    given(args, name).ok_or_else(|| Failure::Refused(format!("{name} is required")))
}

fn accepted(reply: Reply) -> Result<Value, Failure> {
    match reply {
        Reply::Ok(data) => Ok(data),
        Reply::Refused(message) => Err(Failure::Refused(message)),
    }
}

/// A value in a Twenty filter: quoted, so without quotes of its own.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', ""))
}

fn contains(field: &str, value: &str) -> String {
    format!("{field}[ilike]:{}", quoted(&format!("%{value}%")))
}

/// `name` against a person's two name fields: "Ann Lee" must match both, "Lee" either.
fn person_name(name: &str) -> String {
    let (first, last) = ("name.firstName", "name.lastName");
    match name.split_once(char::is_whitespace) {
        Some((a, b)) => format!("{},{}", contains(first, a), contains(last, b.trim())),
        None => format!("or({},{})", contains(first, name), contains(last, name)),
    }
}

fn filter(parts: Vec<String>) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", PAGE.to_string()), ("depth", "1".to_string())];
    if !parts.is_empty() {
        query.push(("filter", parts.join(",")));
    }
    query
}

/// The records of list `name` in an answer (absent means none).
fn records(data: &Value, name: &str) -> Vec<Value> {
    data[name].as_array().cloned().unwrap_or_default()
}

/// The id in a `create<Name>` answer.
fn created_id(data: &Value, name: &str) -> Result<String, Failure> {
    let id = data[format!("create{name}")]["id"].as_str();
    id.map(str::to_string)
        .ok_or_else(|| Failure::Down(anyhow::anyhow!("Twenty's answer to create{name} has no id")))
}

fn day(v: &Value) -> Value {
    v.as_str()
        .map_or(Value::Null, |d| json!(&d[..d.len().min(10)]))
}

fn opportunity_view(o: &Value) -> Value {
    let micros = o["amount"]["amountMicros"].as_f64();
    json!({"id": o["id"], "name": o["name"], "stage": o["stage"], "close_date": day(&o["closeDate"]),
           "amount": micros.map(|m| m / 1e6), "currency": o["amount"]["currencyCode"],
           "account_id": o["companyId"]})
}

fn contact_view(p: &Value) -> Value {
    let name = format!(
        "{} {}",
        p["name"]["firstName"].as_str().unwrap_or_default(),
        p["name"]["lastName"].as_str().unwrap_or_default()
    );
    json!({"id": p["id"], "name": name.trim(), "email": p["emails"]["primaryEmail"],
           "title": p["jobTitle"], "account_id": p["companyId"]})
}

fn account_view(c: &Value) -> Value {
    let contacts: Vec<Value> = records(c, "people").iter().map(contact_view).collect();
    let deals: Vec<Value> = records(c, "opportunities")
        .iter()
        .map(opportunity_view)
        .collect();
    json!({"id": c["id"], "name": c["name"], "domain": c["domainName"]["primaryLinkUrl"],
           "contacts": contacts, "opportunities": deals})
}

fn keys(kind: &str, list: &[Value]) -> Vec<String> {
    let ids = list.iter().filter_map(|v| v["id"].as_str());
    ids.map(|id| format!("{kind}/{id}")).collect()
}

/// A search's answer: `views` under `name`, having read `kind/<id>` for each.
fn found(name: &str, kind: &str, views: Vec<Value>) -> ToolOutput {
    ToolOutput {
        reads: keys(kind, &views),
        result: Some(json!({name: views})),
        ..ToolOutput::default()
    }
}

async fn find_account(rest: &Rest, args: &Value) -> Out {
    let parts = given(args, "name").map(|n| contains("name", n));
    let query = filter(parts.into_iter().collect());
    let data = accepted(rest.get("companies", &query).await?)?;
    let views = records(&data, "companies")
        .iter()
        .map(account_view)
        .collect();
    Ok(found("accounts", "company", views))
}

fn equals(field: &str, value: &str) -> String {
    format!("{field}[eq]:{}", quoted(value))
}

async fn find_contact(rest: &Rest, args: &Value) -> Out {
    let mut parts = vec![];
    parts.extend(given(args, "name").map(person_name));
    parts.extend(given(args, "email").map(|e| contains("emails.primaryEmail", e)));
    parts.extend(given(args, "account_id").map(|a| equals("companyId", a)));
    let data = accepted(rest.get("people", &filter(parts)).await?)?;
    let views = records(&data, "people").iter().map(contact_view).collect();
    Ok(found("contacts", "person", views))
}

async fn find_opportunities(rest: &Rest, args: &Value) -> Out {
    let mut parts = vec![];
    parts.extend(given(args, "name").map(|n| contains("name", n)));
    parts.extend(given(args, "account_id").map(|a| equals("companyId", a)));
    let data = accepted(rest.get("opportunities", &filter(parts)).await?)?;
    let views = records(&data, "opportunities")
        .iter()
        .map(opportunity_view)
        .collect();
    Ok(found("opportunities", "opportunity", views))
}

fn markdown(text: &str) -> Value {
    json!({"markdown": text, "blocknote": null})
}

/// Refuses an account that doesn't exist, before anything is written for it.
async fn check_account(rest: &Rest, id: &str) -> Result<(), Failure> {
    accepted(rest.get(&format!("companies/{id}"), &[]).await?).map(drop)
}

async fn log_note(rest: &Rest, args: &Value) -> Out {
    let account = required(args, "account_id")?;
    let note = json!({"title": required(args, "title")?,
                      "bodyV2": markdown(required(args, "body")?)});
    check_account(rest, account).await?;
    let id = created_id(&accepted(rest.post("notes", &note).await?)?, "Note")?;
    let target = json!({"noteId": id, "targetCompanyId": account});
    let link = accepted(rest.post("noteTargets", &target).await?)?;
    let link = created_id(&link, "NoteTarget")?;
    Ok(ToolOutput {
        result: Some(json!({"id": id, "account_id": account})),
        writes: vec![format!("note/{id}"), format!("noteTarget/{link}")],
        ..ToolOutput::default()
    })
}

async fn update_opportunity(rest: &Rest, args: &Value) -> Out {
    let id = required(args, "id")?;
    let mut fields = json!({});
    if let Some(stage) = given(args, "stage") {
        fields["stage"] = json!(stage);
    }
    if let Some(date) = given(args, "close_date") {
        fields["closeDate"] = json!(date);
    }
    if fields.as_object().is_some_and(|f| f.is_empty()) {
        return Err(Failure::Refused("give a stage or a close_date".into()));
    }
    let data = rest.patch(&format!("opportunities/{id}"), &fields).await?;
    let data = accepted(data)?;
    Ok(ToolOutput {
        result: Some(opportunity_view(&data["updateOpportunity"])),
        writes: vec![format!("opportunity/{id}")],
        ..ToolOutput::default()
    })
}

async fn create_task(rest: &Rest, args: &Value) -> Out {
    let mut task = json!({"title": required(args, "title")?, "status": "TODO"});
    if let Some(body) = given(args, "body") {
        task["bodyV2"] = markdown(body);
    }
    if let Some(due) = given(args, "due") {
        task["dueAt"] = json!(due);
    }
    let account = given(args, "account_id");
    if let Some(account) = account {
        check_account(rest, account).await?;
    }
    let id = created_id(&accepted(rest.post("tasks", &task).await?)?, "Task")?;
    let mut writes = vec![format!("task/{id}")];
    if let Some(account) = account {
        let target = json!({"taskId": id, "targetCompanyId": account});
        let link = accepted(rest.post("taskTargets", &target).await?)?;
        writes.push(format!("taskTarget/{}", created_id(&link, "TaskTarget")?));
    }
    Ok(ToolOutput {
        result: Some(json!({"id": id})),
        writes,
        ..ToolOutput::default()
    })
}

/// Runs tool `name`.
pub async fn call(rest: &Rest, name: &str, args: &Value) -> Result<ToolOutput, PartError> {
    let outcome = match name {
        "crm_find_account" => find_account(rest, args).await,
        "crm_find_contact" => find_contact(rest, args).await,
        "crm_find_opportunities" => find_opportunities(rest, args).await,
        "crm_log_note" => log_note(rest, args).await,
        "crm_update_opportunity" => update_opportunity(rest, args).await,
        "crm_create_task" => create_task(rest, args).await,
        other => return Err(PartError::invalid(format!("no tool {other}"))),
    };
    match outcome {
        Ok(output) => Ok(output),
        Err(Failure::Refused(message)) => Ok(ToolOutput {
            error: Some(message),
            ..ToolOutput::default()
        }),
        Err(Failure::Down(e)) => Err(PartError::internal(format!("{e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_quote_their_values() {
        assert_eq!(contains("name", "No\"rth"), "name[ilike]:\"%North%\"");
        assert_eq!(equals("companyId", "x"), "companyId[eq]:\"x\"");
    }

    #[test]
    fn a_full_name_matches_both_fields_and_one_word_either() {
        assert_eq!(
            person_name("Ann Lee"),
            "name.firstName[ilike]:\"%Ann%\",name.lastName[ilike]:\"%Lee%\""
        );
        assert!(person_name("Lee").starts_with("or("));
    }

    #[test]
    fn views_give_the_fields_an_agent_needs() {
        let o = json!({"id": "o1", "name": "A-1", "stage": "PROPOSAL", "companyId": "c1",
                       "closeDate": "2026-11-05T06:00:00.000Z",
                       "amount": {"amountMicros": 57600000000u64, "currencyCode": "USD"}});
        let c = json!({"id": "c1", "name": "N", "domainName": {"primaryLinkUrl": "https://n.example"},
                       "opportunities": [o],
                       "people": [{"id": "p1", "name": {"firstName": "A", "lastName": "C"},
                                   "emails": {"primaryEmail": "a@n.example"}, "jobTitle": "Buyer"}]});
        let v = account_view(&c);
        assert_eq!(v["domain"], "https://n.example");
        assert_eq!(v["opportunities"][0]["close_date"], "2026-11-05");
        assert_eq!(v["opportunities"][0]["amount"], 57600.0);
        assert_eq!(v["contacts"][0]["name"], "A C");
    }

    #[tokio::test]
    async fn bad_arguments_are_the_tools_error_and_touch_nothing() {
        let rest = Rest::new(1, "k").unwrap();
        let out = call(&rest, "crm_log_note", &json!({"title": "t"}))
            .await
            .unwrap();
        assert_eq!(out.error.as_deref(), Some("account_id is required"));
        let out = call(&rest, "crm_update_opportunity", &json!({"id": "x"})).await;
        assert_eq!(
            out.unwrap().error.as_deref(),
            Some("give a stage or a close_date")
        );
        assert!(call(&rest, "nope", &json!({})).await.is_err());
    }

    #[tokio::test]
    async fn twenty_not_answering_fails_the_call() {
        let rest = Rest::new(1, "k").unwrap();
        assert!(call(&rest, "crm_find_account", &json!({})).await.is_err());
    }
}
