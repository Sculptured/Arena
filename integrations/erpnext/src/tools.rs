//! The tools: documents read and written through Frappe's REST API. Keys are
//! `<Doctype>/<name>`. Frappe refusing a call (a missing document, a failed validation) is the
//! tool's error, not the integration's.
use arena_part::proto::ToolOutput;
use arena_part::PartError;
use reqwest::Method;
use serde_json::{json, Map, Value};

use crate::names::key;
use crate::rest::{Reply, Rest};

/// How many documents `list_docs` gives unless asked otherwise.
const DEFAULT_LIMIT: u64 = 20;

fn text<'a>(args: &'a Value, field: &str) -> Result<&'a str, String> {
    match args.get(field).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(format!("{field} is required (text)")),
    }
}

fn object<'a>(args: &'a Value, field: &str) -> Result<&'a Map<String, Value>, String> {
    args.get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} is required (an object)"))
}

fn failed(error: String, reads: Vec<String>) -> ToolOutput {
    ToolOutput {
        result: None,
        error: Some(error),
        reads,
        writes: vec![],
    }
}

/// Runs tool `name` on `site`.
pub async fn call(
    rest: &Rest,
    site: &str,
    name: &str,
    args: &Value,
) -> Result<ToolOutput, PartError> {
    let out = match name {
        "get_doc" => get_doc(rest, site, args).await?,
        "list_docs" => list_docs(rest, site, args).await?,
        "insert_doc" => insert_doc(rest, site, args).await?,
        "update_doc" => update_doc(rest, site, args).await?,
        "submit_doc" => submit_doc(rest, site, args).await?,
        _ => Err(failed(format!("unknown tool {name}"), vec![])),
    };
    Ok(out.unwrap_or_else(|o| o))
}

/// A tool's outcome: `Ok` its output, `Err` its output when it failed.
type Outcome = Result<ToolOutput, ToolOutput>;

fn doc_name(args: &Value) -> Result<(&str, &str), ToolOutput> {
    let pair = text(args, "doctype").and_then(|d| Ok((d, text(args, "name")?)));
    pair.map_err(|e| failed(e, vec![]))
}

async fn get_doc(rest: &Rest, site: &str, args: &Value) -> Result<Outcome, PartError> {
    let (doctype, name) = match doc_name(args) {
        Ok(p) => p,
        Err(o) => return Ok(Err(o)),
    };
    let reads = vec![key(doctype, name)];
    Ok(
        match rest.get(site, &["resource", doctype, name], &[]).await? {
            Reply::Ok(doc) => Ok(ToolOutput {
                result: Some(doc),
                error: None,
                reads,
                writes: vec![],
            }),
            Reply::Refused(e) => Err(failed(e, reads)),
        },
    )
}

/// `fields` with `name` added (so every row has a key), as JSON text; `["name"]` by default.
fn list_fields(args: &Value) -> Result<String, String> {
    let mut fields: Vec<String> = match args.get("fields") {
        None | Some(Value::Null) => vec![],
        Some(v) => {
            serde_json::from_value(v.clone()).map_err(|_| "fields must be a list of field names")?
        }
    };
    if !fields.iter().any(|f| f == "name" || f == "*") {
        fields.push("name".into());
    }
    Ok(json!(fields).to_string())
}

fn list_query(args: &Value) -> Result<Vec<(&'static str, String)>, String> {
    let limit = match args.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => v.as_u64().ok_or("limit must be a whole number")?,
    };
    let mut q = vec![
        ("fields", list_fields(args)?),
        ("limit_page_length", limit.to_string()),
    ];
    if let Some(f) = args.get("filters").filter(|f| !f.is_null()) {
        q.push(("filters", f.to_string()));
    }
    Ok(q)
}

async fn list_docs(rest: &Rest, site: &str, args: &Value) -> Result<Outcome, PartError> {
    let query = text(args, "doctype").and_then(|d| Ok((d, list_query(args)?)));
    let (doctype, query) = match query {
        Ok(q) => q,
        Err(e) => return Ok(Err(failed(e, vec![]))),
    };
    Ok(
        match rest.get(site, &["resource", doctype], &query).await? {
            Reply::Ok(rows) => {
                let reads = row_keys(doctype, &rows);
                Ok(ToolOutput {
                    result: Some(rows),
                    error: None,
                    reads,
                    writes: vec![],
                })
            }
            Reply::Refused(e) => Err(failed(e, vec![])),
        },
    )
}

/// The keys of the listed rows that have a name.
fn row_keys(doctype: &str, rows: &Value) -> Vec<String> {
    let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str))
        .map(|n| key(doctype, n))
        .collect()
}

/// A write's output: the document Frappe gives back, and its key.
fn written(doctype: &str, reply: Reply, known: Option<&str>) -> Outcome {
    match reply {
        Reply::Ok(doc) => {
            let name = doc
                .get("name")
                .and_then(Value::as_str)
                .or(known)
                .unwrap_or_default();
            let writes = vec![key(doctype, name)];
            Ok(ToolOutput {
                result: Some(doc),
                error: None,
                reads: vec![],
                writes,
            })
        }
        Reply::Refused(e) => Err(failed(
            e,
            known.map(|n| key(doctype, n)).into_iter().collect(),
        )),
    }
}

async fn insert_doc(rest: &Rest, site: &str, args: &Value) -> Result<Outcome, PartError> {
    let doc = object(args, "doc").and_then(|d| Ok((d, text(&args["doc"], "doctype")?)));
    let (doc, doctype) = match doc {
        Ok(d) => d,
        Err(e) => return Ok(Err(failed(e, vec![]))),
    };
    let body = Value::Object(doc.clone());
    let reply = rest
        .send(Method::POST, site, &["resource", doctype], &body)
        .await?;
    Ok(written(doctype, reply, None))
}

/// Sets `body`'s fields on the document (Frappe submits it when `docstatus` goes to 1).
async fn put(
    rest: &Rest,
    site: &str,
    doctype: &str,
    name: &str,
    body: &Value,
) -> Result<Outcome, PartError> {
    let reply = rest
        .send(Method::PUT, site, &["resource", doctype, name], body)
        .await?;
    Ok(written(doctype, reply, Some(name)))
}

async fn update_doc(rest: &Rest, site: &str, args: &Value) -> Result<Outcome, PartError> {
    let parsed = doc_name(args)
        .and_then(|p| Ok((p, object(args, "fields").map_err(|e| failed(e, vec![]))?)));
    match parsed {
        Ok(((doctype, name), fields)) => {
            put(rest, site, doctype, name, &Value::Object(fields.clone())).await
        }
        Err(o) => Ok(Err(o)),
    }
}

async fn submit_doc(rest: &Rest, site: &str, args: &Value) -> Result<Outcome, PartError> {
    match doc_name(args) {
        Ok((doctype, name)) => put(rest, site, doctype, name, &json!({"docstatus": 1})).await,
        Err(o) => Ok(Err(o)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_queries_always_ask_for_names() {
        let q = list_query(&json!({"doctype": "Item"})).unwrap();
        assert_eq!(
            q,
            [
                ("fields", "[\"name\"]".to_string()),
                ("limit_page_length", "20".into())
            ]
        );
        let q =
            list_query(&json!({"fields": ["item_name"], "limit": 0, "filters": {"a": 1}})).unwrap();
        assert_eq!(q[0].1, "[\"item_name\",\"name\"]");
        assert_eq!(q[1].1, "0");
        assert_eq!(q[2], ("filters", "{\"a\":1}".to_string()));
        assert!(list_query(&json!({"fields": "x"})).is_err());
        assert!(list_query(&json!({"limit": -1})).is_err());
    }

    #[test]
    fn writes_name_the_document_frappe_gave_back() {
        let out = written("ToDo", Reply::Ok(json!({"name": "abc12"})), None).unwrap();
        assert_eq!(out.writes, ["ToDo/abc12"]);
        let out = written("Item", Reply::Refused("no".into()), Some("I-1")).unwrap_err();
        assert_eq!(
            (out.error.as_deref(), out.reads),
            (Some("no"), vec!["Item/I-1".to_string()])
        );
        assert!(out.writes.is_empty());
        assert_eq!(
            row_keys("Item", &json!([{"name": "A"}, {"x": 1}])),
            ["Item/A"]
        );
        assert_eq!(
            doc_name(&json!({"doctype": "Item"}))
                .unwrap_err()
                .error
                .unwrap(),
            "name is required (text)"
        );
    }
}
