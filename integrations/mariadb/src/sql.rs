//! Running the `sql_query` and `sql_execute` tools, and turning rows into JSON.
use arena_part::helpers::mariadb::mysql_async::consts::ColumnType;
use arena_part::helpers::mariadb::mysql_async::prelude::Queryable;
use arena_part::helpers::mariadb::mysql_async::{self as my, Conn, Params, Row};
use arena_part::helpers::sql_tools::schemas;
use arena_part::proto::ToolSchema;
use serde_json::{json, Map, Value};

use crate::keys::{changes_default_db, read_only};

pub fn tools() -> Vec<ToolSchema> {
    schemas("values for the ? placeholders, in order")
}

fn param(v: &Value) -> my::Value {
    match v {
        Value::Null => my::Value::NULL,
        Value::Bool(b) => my::Value::Int(i64::from(*b)),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => my::Value::Int(i),
            (None, Some(u)) => my::Value::UInt(u),
            _ => my::Value::Double(n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(s) => my::Value::Bytes(s.clone().into_bytes()),
        other => my::Value::Bytes(other.to_string().into_bytes()),
    }
}

fn params(vals: &[Value]) -> Params {
    if vals.is_empty() {
        Params::Empty
    } else {
        Params::Positional(vals.iter().map(param).collect())
    }
}

fn date_text(c: ColumnType, (y, mo, d, h, mi, s, us): (u16, u8, u8, u8, u8, u8, u32)) -> String {
    let date = format!("{y:04}-{mo:02}-{d:02}");
    if c == ColumnType::MYSQL_TYPE_DATE {
        return date;
    }
    let frac = if us > 0 {
        format!(".{us:06}")
    } else {
        String::new()
    };
    format!("{date} {h:02}:{mi:02}:{s:02}{frac}")
}

fn time_text(neg: bool, d: u32, h: u8, m: u8, s: u8, us: u32) -> String {
    let sign = if neg { "-" } else { "" };
    let frac = if us > 0 {
        format!(".{us:06}")
    } else {
        String::new()
    };
    format!("{sign}{:02}:{m:02}:{s:02}{frac}", d * 24 + u32::from(h))
}

/// One cell as JSON: numbers as numbers, dates and times as text, everything else as text.
fn cell(v: my::Value, c: ColumnType) -> Value {
    match v {
        my::Value::NULL => Value::Null,
        my::Value::Int(i) => json!(i),
        my::Value::UInt(u) => json!(u),
        my::Value::Float(f) => json!(f),
        my::Value::Double(f) => json!(f),
        my::Value::Bytes(b) => Value::String(String::from_utf8_lossy(&b).into_owned()),
        my::Value::Date(y, mo, d, h, mi, s, us) => json!(date_text(c, (y, mo, d, h, mi, s, us))),
        my::Value::Time(neg, d, h, m, s, us) => json!(time_text(neg, d, h, m, s, us)),
    }
}

/// A row as a JSON object of column name to value.
pub fn row_json(row: Row) -> Map<String, Value> {
    let cols = row.columns();
    let vals = row.unwrap();
    cols.iter()
        .zip(vals)
        .map(|(c, v)| (c.name_str().into_owned(), cell(v, c.column_type())))
        .collect()
}

/// Rows of a prepared statement (so values keep their types).
pub async fn rows(
    c: &mut Conn,
    sql: &str,
    vals: &[Value],
) -> Result<Vec<Map<String, Value>>, my::Error> {
    let rows: Vec<Row> = c.exec(sql, params(vals)).await?;
    Ok(rows.into_iter().map(row_json).collect())
}

/// `sql_query`: the rows as a JSON list. Only one reading statement, run in a read-only
/// transaction that is rolled back.
pub async fn query(c: &mut Conn, sql: &str, vals: &[Value]) -> Result<Value, String> {
    if !read_only(sql) {
        return Err("sql_query runs one SELECT, WITH, SHOW, DESCRIBE or EXPLAIN; use sql_execute to change data".into());
    }
    c.query_drop("START TRANSACTION READ ONLY")
        .await
        .map_err(|e| e.to_string())?;
    let got = rows(c, sql, vals).await;
    let rolled_back = c.query_drop("ROLLBACK").await;
    let got = got.map_err(|e| e.to_string())?;
    rolled_back.map_err(|e| e.to_string())?;
    Ok(Value::Array(got.into_iter().map(Value::Object).collect()))
}

/// `sql_execute`: without params any statements (several, `;`-separated, may run); with params
/// one prepared statement.
pub async fn execute(c: &mut Conn, sql: &str, vals: &[Value]) -> Result<Value, String> {
    if changes_default_db(sql) {
        return Err(
            "sql_execute can't change the default database (USE): a save holds only the \
                    instance's own database"
                .into(),
        );
    }
    let done = if vals.is_empty() {
        c.query_drop(sql).await
    } else {
        c.exec_drop(sql, params(vals)).await
    };
    done.map_err(|e| e.to_string())?;
    Ok(json!({"rows_affected": c.affected_rows()}))
}
