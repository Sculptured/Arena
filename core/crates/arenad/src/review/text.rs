//! Small text helpers for digests: cutting long text and JSON to a bound, on char boundaries.

use serde_json::Value;

/// `s` cut to at most `max` bytes (on a char boundary), with `…` and the bytes left out when cut.
pub fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    format!("{}… [{} more bytes]", &s[..i], s.len() - i)
}

/// `v` as compact JSON, cut to `max` bytes; a string is shown as itself, not quoted.
pub fn cut_json(v: &Value, max: usize) -> String {
    match v {
        Value::String(s) => cut(s, max),
        other => cut(&other.to_string(), max),
    }
}

/// `s` with runs of whitespace (newlines included) made one space, then cut to `max` bytes.
pub fn one_line(s: &str, max: usize) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&joined, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cuts_on_char_boundaries_and_says_how_much_was_left_out() {
        assert_eq!(cut("short", 10), "short");
        assert_eq!(cut("abcdef", 3), "abc… [3 more bytes]");
        // "é" is two bytes: a cut inside it moves back.
        assert_eq!(cut("aé", 2), "a… [2 more bytes]");
        assert_eq!(cut_json(&json!("text"), 10), "text");
        assert_eq!(cut_json(&json!({"k": 1}), 100), "{\"k\":1}");
        assert_eq!(one_line("a\n\n  b\tc", 100), "a b c");
    }
}
