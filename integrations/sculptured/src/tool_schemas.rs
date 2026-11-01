//! What the tools take, as `describe` shows them. The arguments of `propose_change` are those of
//! Sculptured's own `propose_change` agent tool.
use arena_part::proto::{Effect, ToolSchema};
use serde_json::{json, Value};

fn tool(name: &str, description: &str, effect: Effect, input: Value) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: input,
        effect,
    }
}

fn propose_change() -> ToolSchema {
    let input = json!({"type": "object", "required": ["action", "summary", "payload"], "properties": {
        "action": {"type": "string", "description": "what kind of change, at most 80 characters (e.g. create_purchase_order)"},
        "summary": {"type": "string", "description": "the change in one line, as a person deciding it reads it"},
        "payload": {"type": "object", "description": "exactly the fields you would write"},
        "system": {"type": "string", "description": "the business system it changes (e.g. ERP)"},
        "reason": {"type": "string", "description": "why the change is needed"}}});
    tool(
        "propose_change",
        "Propose a change to a business system (an order, a schedule, a record in the ERP) before \
         making it. The change waits for a decision; carry it out only once it is approved, \
         exactly as proposed.",
        Effect::Write,
        input,
    )
}

fn list_changes() -> ToolSchema {
    tool(
        "list_changes",
        "The team's proposed changes, newest first: each with its id, action, payload and \
         decision ({approved, by, note}, or null while it waits).",
        Effect::Read,
        json!({"type": "object", "properties": {}}),
    )
}

fn decide_change() -> ToolSchema {
    let input = json!({"type": "object", "required": ["id", "approved"], "properties": {
        "id": {"type": "string", "description": "the change's id"},
        "approved": {"type": "boolean", "description": "true to approve it, false to refuse it"},
        "note": {"type": "string", "description": "why it is refused (only with approved: false)"}}});
    tool(
        "decide_change",
        "Approve or refuse a proposed change that waits for a decision, as the person who \
         decides.",
        Effect::Write,
        input,
    )
}

pub fn schemas() -> Vec<ToolSchema> {
    vec![propose_change(), list_changes(), decide_change()]
}
