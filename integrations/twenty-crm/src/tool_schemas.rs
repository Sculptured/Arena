//! The tools' schemas.
use arena_part::proto::{Effect, ToolSchema};
use serde_json::{json, Value};

/// Each tool: name, description, effect and required arguments.
const TOOLS: [(&str, &str, Effect, &[&str]); 6] = [
    (
        "crm_find_account",
        "Find CRM accounts (companies) by name, with their contacts and opportunities.",
        Effect::Read,
        &[],
    ),
    (
        "crm_find_contact",
        "Find CRM contacts by name, email or account.",
        Effect::Read,
        &[],
    ),
    (
        "crm_find_opportunities",
        "Find CRM opportunities by account or name.",
        Effect::Read,
        &[],
    ),
    (
        "crm_log_note",
        "Log a note on a CRM account: what happened, as the account's activity.",
        Effect::Write,
        &["account_id", "title", "body"],
    ),
    (
        "crm_update_opportunity",
        "Change a CRM opportunity's stage or close date.",
        Effect::Write,
        &["id"],
    ),
    (
        "crm_create_task",
        "Create a CRM task, on an account if one is given.",
        Effect::Write,
        &["title"],
    ),
];

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

/// The arguments of tool `name`.
fn properties(name: &str) -> Value {
    let account = text("the account's id, from crm_find_account");
    let part = "any part of it, ignoring case";
    match name {
        "crm_find_account" => json!({"name": text(part)}),
        "crm_find_contact" => json!({"name": text(part), "email": text(part),
                                     "account_id": account}),
        "crm_find_opportunities" => json!({"name": text(part), "account_id": account}),
        "crm_log_note" => json!({"account_id": account, "title": text("a short title"),
                                 "body": text("the note, in Markdown")}),
        "crm_update_opportunity" => json!({
            "id": text("the opportunity's id"),
            "stage": {"type": "string",
                      "enum": ["NEW", "SCREENING", "MEETING", "PROPOSAL", "CUSTOMER"]},
            "close_date": text("the new close date, YYYY-MM-DD"),
        }),
        _ => json!({"title": text("what to do"), "body": text("details, in Markdown"),
                    "due": text("the due date, YYYY-MM-DD"), "account_id": account}),
    }
}

pub fn schemas() -> Vec<ToolSchema> {
    TOOLS
        .iter()
        .map(|&(name, description, effect, required)| ToolSchema {
            name: name.into(),
            description: description.into(),
            input_schema: json!({"type": "object", "properties": properties(name),
                                 "required": required}),
            effect,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_reads_and_three_writes_with_their_arguments() {
        let s = schemas();
        let writes: Vec<_> = s.iter().filter(|t| t.effect == Effect::Write).collect();
        assert_eq!((s.len(), writes.len()), (6, 3));
        let note = s.iter().find(|t| t.name == "crm_log_note").unwrap();
        assert_eq!(note.input_schema["required"][0], "account_id");
        let update = s
            .iter()
            .find(|t| t.name == "crm_update_opportunity")
            .unwrap();
        assert!(update.input_schema["properties"]["close_date"].is_object());
    }
}
