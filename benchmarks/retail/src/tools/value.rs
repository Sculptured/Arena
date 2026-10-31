//! Small helpers on JSON documents and numbers, shared by the tools.

use serde_json::{json, Value};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub fn fold(text: &str) -> String {
    text.trim()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

pub(super) fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}
pub(super) fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}
pub(super) fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}
pub(super) fn is_gift_card(method: &Value) -> bool {
    method["source"] == "gift_card"
}
pub(super) fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(st).collect())
        .unwrap_or_default()
}
pub(super) fn count(v: &[String], x: &str) -> usize {
    v.iter().filter(|y| *y == x).count()
}
/// `round(0, 2)` and friends stay integers in Python when nothing was added.
pub(super) fn money(x: f64, any: bool) -> Value {
    if any {
        json!(x)
    } else {
        json!(0)
    }
}

/// `json.dumps(s)` with Python's default `ensure_ascii`.
pub(super) fn py_json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
