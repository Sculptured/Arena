//! Reading a database's rows by key for `inspect` and `diff`.
use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use arena_part::helpers::folder::hex;
use arena_part::helpers::mariadb::mysql_async::prelude::Queryable;
use arena_part::helpers::mariadb::mysql_async::Conn;
use arena_part::helpers::mariadb::quote_ident;
use arena_part::helpers::sql_tools::{lookup, KeyLookup};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::sql::rows;

/// Each table with a one-column primary key, and that column.
pub async fn pk_columns(c: &mut Conn) -> Result<HashMap<String, String>> {
    let sql = "SELECT TABLE_NAME, MIN(COLUMN_NAME) FROM information_schema.KEY_COLUMN_USAGE \
               WHERE TABLE_SCHEMA = DATABASE() AND CONSTRAINT_NAME = 'PRIMARY' \
               GROUP BY TABLE_NAME HAVING COUNT(*) = 1";
    Ok(c.query::<(String, String), _>(sql)
        .await?
        .into_iter()
        .collect())
}

async fn base_tables(c: &mut Conn) -> Result<Vec<String>> {
    let sql = "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() \
               AND TABLE_TYPE = 'BASE TABLE' ORDER BY 1";
    Ok(c.query(sql).await?)
}

/// A key's id: a primary key value as text.
fn id_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Every row of every table with a one-column primary key, keyed `<table>/<pk>`.
pub async fn all_rows(c: &mut Conn) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for (table, pk) in pk_columns(c).await? {
        for row in rows(c, &format!("SELECT * FROM {}", quote_ident(&table)), &[]).await? {
            let id = row.get(&pk).map(id_text).unwrap_or_default();
            out.insert(format!("{table}/{id}"), Value::Object(row));
        }
    }
    Ok(out)
}

/// A hash of the rows of each table without a one-column primary key, keyed `<table>/*`.
async fn whole_tables(c: &mut Conn) -> Result<BTreeMap<String, Value>> {
    let pks = pk_columns(c).await?;
    let mut out = BTreeMap::new();
    for table in base_tables(c).await? {
        if pks.contains_key(&table) {
            continue;
        }
        let all = rows(c, &format!("SELECT * FROM {}", quote_ident(&table)), &[]).await?;
        let mut lines: Vec<String> = all
            .into_iter()
            .map(|r| Value::Object(r).to_string())
            .collect();
        lines.sort();
        let hash = Sha256::digest(lines.join("\n").as_bytes());
        out.insert(format!("{table}/*"), json!(hex(&hash)));
    }
    Ok(out)
}

/// What `diff` compares: every row by key, plus a hash of each table without a one-column key.
pub async fn comparable(c: &mut Conn) -> Result<BTreeMap<String, Value>> {
    let mut all = all_rows(c).await?;
    all.extend(whole_tables(c).await?);
    Ok(all)
}

/// The row of one key, or null.
async fn one_row(c: &mut Conn, table: &str, pk: &str, id: &str) -> Result<Value> {
    let sql = format!(
        "SELECT * FROM {} WHERE CAST({} AS CHAR) = ?",
        quote_ident(table),
        quote_ident(pk)
    );
    let found = rows(c, &sql, &[json!(id)]).await?.into_iter().next();
    Ok(found.map_or(Value::Null, Value::Object))
}

/// The docs of `keys`; `Ok(Err(message))` when a key names a table without a one-column key.
pub async fn some_rows(
    c: &mut Conn,
    keys: &[String],
) -> Result<Result<Map<String, Value>, String>> {
    let pks = pk_columns(c).await?;
    let tables = base_tables(c).await?;
    let mut out = Map::new();
    for k in keys {
        let doc = match lookup(k, &pks, |t| tables.iter().any(|x| x == t)) {
            Err(m) => return Ok(Err(m)),
            Ok(KeyLookup::Missing) => Value::Null,
            Ok(KeyLookup::Row { table, pk, id }) => one_row(c, table, pk, id).await?,
        };
        out.insert(k.clone(), doc);
    }
    Ok(Ok(out))
}
