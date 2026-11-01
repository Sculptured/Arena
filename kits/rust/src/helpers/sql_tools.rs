//! What SQL integrations share: the `sql_query` and `sql_execute` tools (schemas, arguments,
//! outputs) and how `inspect` keys map to rows.
use std::collections::HashMap;

use serde_json::{json, Value};

use crate::proto::{Effect, ToolOutput, ToolSchema};

/// Which of the two tools a call names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlTool {
    /// `sql_query`: rows, reported as reads.
    Query,
    /// `sql_execute`: rows affected, reported as writes.
    Execute,
}

fn schema(name: &str, description: &str, effect: Effect, params_hint: &str) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "sql": {"type": "string"},
                "params": {"type": "array", "items": {}, "description": params_hint}
            },
            "required": ["sql"]
        }),
        effect,
    }
}

/// The two tools; `params_hint` says how the dialect writes placeholders.
pub fn schemas(params_hint: &str) -> Vec<ToolSchema> {
    vec![
        schema(
            "sql_query",
            "Run a SELECT and get the rows as JSON objects.",
            Effect::Read,
            params_hint,
        ),
        schema(
            "sql_execute",
            "Run an INSERT, UPDATE, DELETE or other statement and get the rows affected.",
            Effect::Write,
            params_hint,
        ),
    ]
}

/// The tool `name` names, when the tools are on; otherwise the output to answer with.
pub fn which(name: &str, enabled: bool) -> Result<SqlTool, ToolOutput> {
    match (name, enabled) {
        ("sql_query", true) => Ok(SqlTool::Query),
        ("sql_execute", true) => Ok(SqlTool::Execute),
        (other, _) => Err(tool_error(format!("unknown tool {other}"))),
    }
}

/// The `sql` and `params` of a tool call.
pub fn sql_args(args: &Value) -> Result<(&str, &[Value]), String> {
    let sql = args
        .get("sql")
        .and_then(Value::as_str)
        .ok_or("sql must be a string")?;
    let params = match args.get("params") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(a)) => a.as_slice(),
        Some(_) => return Err("params must be a list".into()),
    };
    Ok((sql, params))
}

/// A tool call that failed in a way the agent should see (not a protocol error).
pub fn tool_error(m: String) -> ToolOutput {
    ToolOutput {
        error: Some(m),
        ..Default::default()
    }
}

/// A tool's output: its result, and the keys it touched as reads or writes.
pub fn tool_output(tool: SqlTool, out: Result<Value, String>, keys: Vec<String>) -> ToolOutput {
    let result = match out {
        Ok(v) => Some(v),
        Err(e) => return tool_error(e),
    };
    let (reads, writes) = match tool {
        SqlTool::Query => (keys, vec![]),
        SqlTool::Execute => (vec![], keys),
    };
    ToolOutput {
        result,
        reads,
        writes,
        error: None,
    }
}

/// Where an `inspect` key's doc comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyLookup<'a> {
    /// The row of `table` whose primary key column `pk` reads as `id`.
    Row {
        table: &'a str,
        pk: &'a str,
        id: &'a str,
    },
    /// No such table or not a `<table>/<id>` key: the doc is null.
    Missing,
}

/// How to look up `key`, given each table's one-column primary key (`pks`) and which names are
/// tables (`is_table`). A table without a one-column key can't be inspected by row: `Err`.
pub fn lookup<'a>(
    key: &'a str,
    pks: &'a HashMap<String, String>,
    is_table: impl Fn(&str) -> bool,
) -> Result<KeyLookup<'a>, String> {
    let Some((table, id)) = key.split_once('/') else {
        return Ok(KeyLookup::Missing);
    };
    match pks.get(table) {
        Some(pk) => Ok(KeyLookup::Row { table, pk, id }),
        None if is_table(table) => Err(no_single_pk(table)),
        None => Ok(KeyLookup::Missing),
    }
}

/// The error for inspecting a table without a one-column primary key.
pub fn no_single_pk(table: &str) -> String {
    format!("table {table} has no single-column primary key; inspect {table}/* is not supported")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_outputs() {
        let a = json!({"sql": "SELECT 1", "params": [1]});
        assert_eq!(sql_args(&a).unwrap(), ("SELECT 1", &[json!(1)][..]));
        assert!(sql_args(&json!({"sql": 1})).is_err());
        assert!(sql_args(&json!({"sql": "x", "params": 2})).is_err());
        assert_eq!(which("sql_query", true), Ok(SqlTool::Query));
        assert!(which("sql_query", false).unwrap_err().error.is_some());
        let w = tool_output(SqlTool::Execute, Ok(json!(1)), vec!["t/1".into()]);
        assert_eq!(
            (w.reads.len(), w.writes, w.result),
            (0, vec!["t/1".to_string()], Some(json!(1)))
        );
        let e = tool_output(SqlTool::Query, Err("bad".into()), vec!["t/*".into()]);
        assert_eq!((e.error.as_deref(), e.reads.len()), (Some("bad"), 0));
    }

    #[test]
    fn key_lookup() {
        let pks: HashMap<String, String> = [("t".into(), "id".into())].into();
        let is_table = |t: &str| t == "t" || t == "np";
        assert_eq!(
            lookup("t/7", &pks, is_table),
            Ok(KeyLookup::Row {
                table: "t",
                pk: "id",
                id: "7"
            })
        );
        assert_eq!(lookup("gone/1", &pks, is_table), Ok(KeyLookup::Missing));
        assert_eq!(lookup("t", &pks, is_table), Ok(KeyLookup::Missing));
        assert!(lookup("np/1", &pks, is_table)
            .unwrap_err()
            .contains("np has no single-column"));
    }
}
