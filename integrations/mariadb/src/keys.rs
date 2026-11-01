//! The keys a statement reads or writes, read out of its text: `<table>/<pk>` for a single-table
//! statement that pins the primary key with `=`, else `<table>/*`, else `*`.
//!
//! Small on purpose: anything it does not recognise gets the wider key, never a narrower one.
use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Num(String),
    Param(usize),
    Sym(char),
}

fn take_while(chars: &[char], i: &mut usize, ok: impl Fn(char) -> bool) -> String {
    let start = *i;
    while *i < chars.len() && ok(chars[*i]) {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

/// A quoted run ending at an unescaped `q` (`''` and `\'` escape it); `i` is past the opening.
fn quoted(chars: &[char], i: &mut usize, q: char) -> String {
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        if c == '\\' && q != '`' && *i < chars.len() {
            out.push(chars[*i]);
            *i += 1;
        } else if c == q && chars.get(*i) == Some(&q) {
            out.push(q);
            *i += 1;
        } else if c == q {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The token starting at `chars[*i]` (not whitespace); `params` counts the `?` seen so far.
fn token(chars: &[char], i: &mut usize, params: &mut usize) -> Tok {
    let c = chars[*i];
    if c.is_ascii_digit() {
        return Tok::Num(take_while(chars, i, |c| {
            c.is_ascii_alphanumeric() || c == '.'
        }));
    }
    if is_word_char(c) {
        return Tok::Word(take_while(chars, i, is_word_char));
    }
    *i += 1;
    match c {
        '`' => Tok::Word(quoted(chars, i, c)),
        '\'' | '"' => Tok::Str(quoted(chars, i, c)),
        '?' => {
            *params += 1;
            Tok::Param(*params - 1)
        }
        _ => Tok::Sym(c),
    }
}

fn tokens(sql: &str) -> Vec<Tok> {
    let chars: Vec<char> = sql.chars().collect();
    let (mut out, mut i, mut params) = (vec![], 0, 0);
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
        } else {
            out.push(token(&chars, &mut i, &mut params));
        }
    }
    out
}

/// A value pinned in the statement: a literal, or the n-th `?` parameter.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Lit(String),
    Param(usize),
}

/// What the statement touches: its tables, the `column = value` pairs that pin rows, and the
/// columns it assigns (an update that assigns the key moves the row).
#[derive(Debug, Default, PartialEq)]
struct Parsed {
    tables: Vec<String>,
    pinned: Vec<(String, Val)>,
    assigned: Vec<String>,
}

struct Cursor {
    toks: Vec<Tok>,
    i: usize,
}

impl Cursor {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x.eq_ignore_ascii_case(w))
    }

    fn eat_word(&mut self, w: &str) -> bool {
        let yes = self.is_word(w);
        self.i += usize::from(yes);
        yes
    }

    fn eat_sym(&mut self, s: char) -> bool {
        let yes = self.peek() == Some(&Tok::Sym(s));
        self.i += usize::from(yes);
        yes
    }

    /// `name` or `db.name`: the last part.
    fn name(&mut self) -> Option<String> {
        let Some(Tok::Word(w)) = self.peek().cloned() else {
            return None;
        };
        self.i += 1;
        if self.eat_sym('.') {
            return self.name();
        }
        Some(w)
    }

    fn value(&mut self) -> Option<Val> {
        let neg = self.eat_sym('-');
        let v = match self.peek()?.clone() {
            Tok::Num(n) if neg => Val::Lit(format!("-{n}")),
            Tok::Num(n) | Tok::Str(n) if !neg => Val::Lit(n),
            Tok::Param(p) if !neg => Val::Param(p),
            _ => return None,
        };
        self.i += 1;
        Some(v)
    }

    /// The statement ends here (an optional `;`, or a `LIMIT`).
    fn at_end(&mut self) -> bool {
        self.eat_sym(';');
        self.peek().is_none() || self.is_word("limit")
    }

    /// `col = value` and nothing more but the end.
    fn sole_pin(&mut self) -> Vec<(String, Val)> {
        let pin = (|| {
            let col = self.name()?;
            self.eat_sym('=').then_some(())?;
            Some((col, self.value()?))
        })();
        pin.filter(|_| self.at_end()).into_iter().collect()
    }

    /// `(a, b, ..)`.
    fn list<T>(&mut self, item: impl Fn(&mut Cursor) -> Option<T>) -> Option<Vec<T>> {
        self.eat_sym('(').then_some(())?;
        let mut out = vec![item(self)?];
        while self.eat_sym(',') {
            out.push(item(self)?);
        }
        self.eat_sym(')').then_some(out)
    }

    /// Skips any of `words` (statement modifiers such as `IGNORE`).
    fn skip_words(&mut self, words: &[&str]) {
        while words.iter().any(|w| self.is_word(w)) {
            self.i += 1;
        }
    }
}

fn parse_select(c: &mut Cursor) -> Option<Parsed> {
    let mut p = Parsed::default();
    let mut joined = false;
    while c.peek().is_some() {
        if c.eat_word("from") || c.eat_word("join") {
            joined |= !p.tables.is_empty();
            p.tables.extend(c.name());
            while c.eat_sym(',') {
                joined = true;
                p.tables.extend(c.name());
            }
        } else if !joined && p.tables.len() == 1 && c.eat_word("where") {
            p.pinned = c.sole_pin();
        } else {
            c.i += 1;
        }
    }
    Some(p)
}

/// One table; `None` (so `*`) when the statement's target can't be read out.
fn one_table(c: &mut Cursor) -> Option<Parsed> {
    Some(Parsed {
        tables: vec![c.name()?],
        ..Parsed::default()
    })
}

/// `INSERT [INTO] t (cols) VALUES (vals)` pins the row; a `REPLACE` may delete rows on any
/// unique key, so it is the whole table.
fn parse_insert(c: &mut Cursor, replace: bool) -> Option<Parsed> {
    c.skip_words(&["low_priority", "delayed", "high_priority", "ignore"]);
    c.eat_word("into");
    let mut p = one_table(c)?;
    if replace {
        return Some(p);
    }
    let cols = c.list(Cursor::name);
    let vals = (c.eat_word("values") || c.eat_word("value"))
        .then(|| c.list(Cursor::value))
        .flatten();
    if let (Some(cols), Some(vals), true) = (cols, vals, c.at_end()) {
        p.pinned = cols.into_iter().zip(vals).collect();
    }
    Some(p)
}

/// `UPDATE t SET .. [WHERE col = v]`; anything else after the table (a join, a second table,
/// an alias) is `*`.
fn parse_update(c: &mut Cursor) -> Option<Parsed> {
    c.skip_words(&["low_priority", "ignore"]);
    let mut p = one_table(c)?;
    if !c.eat_word("set") {
        return None;
    }
    while let Some(col) = c.name() {
        p.assigned.push(col);
        while c.peek().is_some() && c.peek() != Some(&Tok::Sym(',')) && !c.is_word("where") {
            c.i += 1;
        }
        if !c.eat_sym(',') {
            break;
        }
    }
    if c.eat_word("where") {
        p.pinned = c.sole_pin();
    }
    Some(p)
}

/// `DELETE FROM t [WHERE col = v | ORDER BY .. | LIMIT ..]`; a multi-table delete is `*`.
fn parse_delete(c: &mut Cursor) -> Option<Parsed> {
    c.skip_words(&["low_priority", "quick", "ignore"]);
    if !c.eat_word("from") {
        return None;
    }
    let mut p = one_table(c)?;
    if c.eat_word("where") {
        p.pinned = c.sole_pin();
    } else if !(c.at_end() || c.is_word("order")) {
        return None;
    }
    Some(p)
}

/// What the statement touches; `None` when it can't be told.
fn parse(toks: Vec<Tok>) -> Option<Parsed> {
    let mut c = Cursor { toks, i: 0 };
    let Some(Tok::Word(first)) = c.peek() else {
        return None;
    };
    let first = first.to_ascii_lowercase();
    c.i += 1;
    match first.as_str() {
        "select" => parse_select(&mut c),
        "insert" => parse_insert(&mut c, false),
        "replace" => parse_insert(&mut c, true),
        "update" => parse_update(&mut c),
        "delete" => parse_delete(&mut c),
        _ => None,
    }
}

/// Whether anything follows a `;`.
fn several(toks: &[Tok]) -> bool {
    toks.iter()
        .position(|t| *t == Tok::Sym(';'))
        .is_some_and(|i| i + 1 < toks.len())
}

/// Whether `sql` is one statement that only reads (SELECT, WITH, SHOW, DESCRIBE, EXPLAIN).
pub fn read_only(sql: &str) -> bool {
    let toks = tokens(sql);
    let reads = ["select", "with", "show", "describe", "desc", "explain"];
    let first = match toks.first() {
        Some(Tok::Word(w)) => reads.iter().any(|r| w.eq_ignore_ascii_case(r)),
        _ => false,
    };
    first && !several(&toks)
}

/// The text of a pinned value, as `inspect` writes it in a key.
fn val_text(v: &Val, params: &[Value]) -> Option<String> {
    match v {
        Val::Lit(s) => Some(s.clone()),
        Val::Param(i) => match params.get(*i)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(u8::from(*b).to_string()),
            _ => None,
        },
    }
}

/// `<table>/*` for each table, sorted; `*` when there are none.
fn whole_tables(tables: &[String]) -> Vec<String> {
    let mut all: Vec<String> = tables.iter().map(|t| format!("{t}/*")).collect();
    all.sort();
    all.dedup();
    if all.is_empty() {
        vec!["*".into()]
    } else {
        all
    }
}

/// The keys `sql` touches, given each table's one-column primary key (`pks`).
pub fn keys(sql: &str, params: &[Value], pks: &HashMap<String, String>) -> Vec<String> {
    let toks = tokens(sql);
    if several(&toks) {
        return vec!["*".into()];
    }
    let Some(p) = parse(toks) else {
        return vec!["*".into()];
    };
    let [table] = p.tables.as_slice() else {
        return whole_tables(&p.tables);
    };
    let is_pk = |col: &str| {
        pks.get(table)
            .is_some_and(|pk| pk.eq_ignore_ascii_case(col))
    };
    let moved = p.assigned.iter().any(|c| is_pk(c));
    let pinned = p.pinned.iter().find(|(c, _)| is_pk(c));
    match pinned
        .filter(|_| !moved)
        .and_then(|(_, v)| val_text(v, params))
    {
        Some(id) => vec![format!("{table}/{id}")],
        None => vec![format!("{table}/*")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn k(sql: &str) -> Vec<String> {
        kp(sql, &[])
    }

    fn kp(sql: &str, params: &[Value]) -> Vec<String> {
        let pks: HashMap<String, String> = [("t".into(), "id".into()), ("u".into(), "code".into())]
            .into_iter()
            .collect();
        keys(sql, params, &pks)
    }

    #[test]
    fn single_row_by_primary_key() {
        assert_eq!(k("INSERT INTO t (id, v) VALUES (1, 'a')"), ["t/1"]);
        assert_eq!(k("insert into `t`(v, id) values ('a', -3);"), ["t/-3"]);
        assert_eq!(k("UPDATE t SET v = 'x, y' WHERE id = 7"), ["t/7"]);
        assert_eq!(k("DELETE FROM db.t WHERE id = 'k''1'"), ["t/k'1"]);
        assert_eq!(k("SELECT * FROM t WHERE ID = 2 LIMIT 1"), ["t/2"]);
        assert_eq!(k("SELECT v FROM u WHERE code = \"A\""), ["u/A"]);
    }

    #[test]
    fn parameters_fill_the_key() {
        let params = [json!("x"), json!(5)];
        assert_eq!(kp("UPDATE t SET v = ? WHERE id = ?", &params), ["t/5"]);
        assert_eq!(kp("INSERT INTO t (v, id) VALUES (?, ?)", &params), ["t/5"]);
        assert_eq!(kp("DELETE FROM t WHERE id = ?", &[Value::Null]), ["t/*"]);
    }

    #[test]
    fn anything_wider_is_the_whole_table() {
        assert_eq!(k("UPDATE t SET v = 'x' WHERE id = 1 OR id = 2"), ["t/*"]);
        assert_eq!(
            k("UPDATE t SET id = 9 WHERE id = 1"),
            ["t/*"],
            "the row moves"
        );
        assert_eq!(k("DELETE FROM t WHERE v = 'a'"), ["t/*"]);
        assert_eq!(k("DELETE FROM t"), ["t/*"]);
        assert_eq!(
            k("INSERT INTO t (id, v) VALUES (1, 'a'), (2, 'b')"),
            ["t/*"]
        );
        assert_eq!(
            k("INSERT INTO t (id, v) VALUES (1, 'a') ON DUPLICATE KEY UPDATE v = 'b'"),
            ["t/*"]
        );
        assert_eq!(k("INSERT INTO t VALUES (1, 'a')"), ["t/*"]);
        assert_eq!(
            k("UPDATE w SET v = 1 WHERE id = 1"),
            ["w/*"],
            "no known key"
        );
    }

    #[test]
    fn statements_that_cannot_be_pinned() {
        assert_eq!(k("UPDATE t SET v = 'a' WHERE id = 1; DELETE FROM u"), ["*"]);
        assert_eq!(
            k("INSERT INTO t (id, v) VALUES (1, 'a');"),
            ["t/1"],
            "a trailing ; is one statement"
        );
        assert_eq!(k("UPDATE t, u SET t.v = u.code WHERE t.id = 1"), ["*"]);
        assert_eq!(
            k("UPDATE t JOIN u ON t.id = u.code SET t.v = 1 WHERE t.id = 1"),
            ["*"]
        );
        assert_eq!(k("DELETE t FROM t JOIN u ON t.id = u.code"), ["*"]);
        assert_eq!(k("DELETE FROM t USING t JOIN u WHERE t.id = 1"), ["*"]);
        assert_eq!(k("REPLACE INTO t (id, v) VALUES (1, 'a')"), ["t/*"]);
        assert_eq!(k("INSERT IGNORE INTO t (id, v) VALUES (4, 'a')"), ["t/4"]);
        assert_eq!(k("UPDATE LOW_PRIORITY t SET v = 1 WHERE id = 2"), ["t/2"]);
        assert_eq!(k("DELETE FROM t ORDER BY id LIMIT 1"), ["t/*"]);
    }

    #[test]
    fn read_only_statements() {
        assert!(read_only("SELECT * FROM t"));
        assert!(read_only(" with x as (select 1) select * from x;"));
        assert!(read_only("SHOW TABLES"));
        assert!(!read_only("SELECT 1; DELETE FROM t"));
        assert!(!read_only("DELETE FROM t"));
        assert!(!read_only("CREATE TABLE z (id int)"));
        assert!(!read_only("/* x */ SELECT 1"));
    }

    #[test]
    fn several_tables_and_unknown_statements() {
        assert_eq!(
            k("SELECT * FROM t JOIN u ON t.id = u.code WHERE t.id = 1"),
            ["t/*", "u/*"]
        );
        assert_eq!(k("SELECT * FROM u, t"), ["t/*", "u/*"]);
        assert_eq!(k("CREATE TABLE z (id int)"), ["*"]);
        assert_eq!(k("SELECT 1"), ["*"]);
    }
}
