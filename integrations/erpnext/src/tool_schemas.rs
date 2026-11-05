//! The tools' schemas.
use arena_part::proto::{Effect, ToolSchema};
use serde_json::{json, Value};

/// Each tool: name, description, effect and required arguments.
const TOOLS: [(&str, &str, Effect, &[&str]); 5] = [
    (
        "get_doc",
        "Read one ERPNext document.",
        Effect::Read,
        &["doctype", "name"],
    ),
    (
        "list_docs",
        "List ERPNext documents of a doctype, a page at a time.",
        Effect::Read,
        &["doctype"],
    ),
    (
        "insert_doc",
        "Create an ERPNext document.",
        Effect::Write,
        &["doc"],
    ),
    (
        "update_doc",
        "Change fields of an ERPNext document.",
        Effect::Write,
        &["doctype", "name", "fields"],
    ),
    (
        "submit_doc",
        "Submit an ERPNext document (docstatus 1).",
        Effect::Write,
        &["doctype", "name"],
    ),
];

/// The arguments of tool `name`.
fn properties(name: &str) -> Value {
    let doctype =
        json!({"type": "string", "description": "the doctype, like Item or Purchase Order"});
    let doc_name = json!({"type": "string", "description": "the document's name (its ID)"});
    match name {
        "list_docs" => json!({
            "doctype": doctype,
            "filters": {"description": "Frappe filters: {field: value} or [[field, op, value]]"},
            "fields": {"type": "array", "items": {"type": "string"},
                       "description": "fields to return (name is always included)"},
            "limit": {"type": "integer", "description": "at most this many (default 20, 0 for all)"},
            "start": {"type": "integer", "description": "skip this many first (for the next page)"},
            "order_by": {"type": "string", "description": "SQL order, like \"modified desc\""},
        }),
        "insert_doc" => {
            json!({"doc": {"type": "object", "description": "the document, with its doctype"}})
        }
        "update_doc" => json!({"doctype": doctype, "name": doc_name,
                               "fields": {"type": "object", "description": "the fields to set"}}),
        _ => json!({"doctype": doctype, "name": doc_name}),
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
    fn five_tools_with_effects_and_arguments() {
        let s = schemas();
        let names: Vec<_> = s.iter().map(|t| (t.name.as_str(), t.effect)).collect();
        assert_eq!(
            names,
            [
                ("get_doc", Effect::Read),
                ("list_docs", Effect::Read),
                ("insert_doc", Effect::Write),
                ("update_doc", Effect::Write),
                ("submit_doc", Effect::Write),
            ]
        );
        assert!(s[3].input_schema["properties"]["fields"].is_object());
        assert_eq!(s[4].input_schema["required"], json!(["doctype", "name"]));
    }
}
