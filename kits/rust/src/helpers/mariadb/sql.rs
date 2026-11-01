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

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Whether `text` names the database `db` as a whole identifier (any case, backticked or not).
pub fn names_db(text: &str, db: &str) -> bool {
    let (text, db) = (text.to_ascii_lowercase(), db.to_ascii_lowercase());
    text.match_indices(&db).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + db.len()..].chars().next();
        !before.is_some_and(ident_char) && !after.is_some_and(ident_char)
    })
}

/// Whether a Query event (its `SHOW BINLOG EVENTS` info: `use `x`; <sql>`, or just `<sql>` with
/// no default database) changes `db` while another default database (or none) was selected.
/// Such an event is not in `db`'s segment, which `mariadb-binlog --database` picks by default
/// database.
pub fn query_misplaced(info: &str, db: &str) -> bool {
    let (current, text) = match info.strip_prefix("use `").and_then(|r| r.split_once("`; ")) {
        Some((current, text)) => (Some(current), text),
        None => (None, info),
    };
    current != Some(db) && names_db(text, db)
}
