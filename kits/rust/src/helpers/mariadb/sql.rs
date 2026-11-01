//! Small SQL text helpers.

/// A backquoted identifier (`a`b` -> `` `a``b` ``).
pub fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// A `LIKE` pattern matching names that start with `prefix`.
pub fn like_prefix(prefix: &str) -> String {
    let mut out = String::new();
    for c in prefix.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// A binlog file name as `SHOW BINARY LOGS` gives it (`binlog.000012`): safe to quote as is.
pub fn valid_binlog_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}
