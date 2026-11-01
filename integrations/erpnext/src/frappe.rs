//! Reading what Frappe answers: error messages, `bench version` and new API keys.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An Administrator API key and its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub api_key: String,
    pub api_secret: String,
}

impl ApiKey {
    /// The `Authorization` header value.
    pub fn header(&self) -> String {
        format!("token {}:{}", self.api_key, self.api_secret)
    }
}

/// The message of a Frappe error response with HTTP status `status`.
pub fn error_text(status: u16, body: &Value) -> String {
    if let Some(m) = server_messages(body) {
        return m;
    }
    for k in ["exception", "exc_type", "message"] {
        if let Some(s) = body
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return s.to_string();
        }
    }
    format!("ERPNext answered HTTP {status}")
}

/// `_server_messages`: a JSON list of JSON objects with a `message`, joined, tags removed.
fn server_messages(body: &Value) -> Option<String> {
    let list: Vec<String> = serde_json::from_str(body.get("_server_messages")?.as_str()?).ok()?;
    let texts: Vec<String> = list
        .iter()
        .filter_map(|m| serde_json::from_str::<Value>(m).ok())
        .filter_map(|m| m.get("message").and_then(Value::as_str).map(strip_tags))
        .collect();
    (!texts.is_empty()).then(|| texts.join("; "))
}

/// `s` without HTML tags.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The app versions `bench version` prints (`erpnext 16.53.0 ()` per line).
pub fn parse_versions(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect()
}

/// The key and secret printed by `generate_keys` (a JSON object on the last line that has one).
pub fn parse_keys(text: &str) -> Option<ApiKey> {
    text.lines()
        .rev()
        .find_map(|l| serde_json::from_str::<ApiKey>(l.trim()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_text_prefers_server_messages() {
        let msg = json!({"message": "Item <strong>NOPE</strong> not found"}).to_string();
        let body = json!({"exc_type": "DoesNotExistError",
            "_server_messages": json!([msg]).to_string()});
        assert_eq!(error_text(404, &body), "Item NOPE not found");
        let body = json!({"exception": "frappe.exceptions.LinkValidationError: Could not find X"});
        assert!(error_text(417, &body).starts_with("frappe.exceptions.LinkValidationError"));
        assert_eq!(error_text(502, &Value::Null), "ERPNext answered HTTP 502");
    }

    #[test]
    fn bench_output_is_parsed() {
        let v = parse_versions("erpnext 16.53.0 ()\nfrappe 16.54.0 ()\n");
        assert_eq!(v["erpnext"], "16.53.0");
        assert_eq!(v["frappe"], "16.54.0");
        let keys = parse_keys("noise\n{\"api_key\": \"k1\", \"api_secret\": \"s1\"}\n").unwrap();
        assert_eq!(keys.header(), "token k1:s1");
        assert_eq!(parse_keys("nothing"), None);
    }
}
