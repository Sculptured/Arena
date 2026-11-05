//! Reading a CRM's records straight from its database, so `inspect` and `diff` work on a save (a
//! database no server runs on) as they do on an instance.
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use arena_part::helpers::pg::tokio_postgres::Client;
use serde_json::Value;

/// The records agents change. Every other table is copied by a save but neither read nor compared.
const TABLES: [&str; 7] = [
    "company",
    "person",
    "opportunity",
    "note",
    "noteTarget",
    "task",
    "taskTarget",
];

/// The tables whose rows are emptied from a new workspace: its demo records, and what refers to them.
pub const DEMO_TABLES: [&str; 8] = [
    "company",
    "person",
    "opportunity",
    "note",
    "noteTarget",
    "task",
    "taskTarget",
    "timelineActivity",
];

/// The accounts a note or task is on, as `accounts`.
fn accounts_of(schema: &str, targets: &str, column: &str) -> String {
    format!(
        "jsonb_build_object('accounts', COALESCE((SELECT jsonb_agg(c.name ORDER BY c.name) \
         FROM {schema}.\"{targets}\" x JOIN {schema}.company c ON c.id = x.\"targetCompanyId\" \
         WHERE x.\"{column}\" = t.id AND x.\"deletedAt\" IS NULL), '[]'::jsonb))"
    )
}

/// What a row's document adds to its columns, so that a check can read a record by its account's name.
fn derived(schema: &str, table: &str) -> String {
    match table {
        "note" => accounts_of(schema, "noteTarget", "noteId"),
        "task" => accounts_of(schema, "taskTarget", "taskId"),
        "opportunity" | "person" => format!(
            "jsonb_build_object('account', (SELECT c.name FROM {schema}.company c \
             WHERE c.id = t.\"companyId\"))"
        ),
        _ => "'{}'::jsonb".to_string(),
    }
}

fn select(schema: &str, table: &str) -> String {
    format!(
        "SELECT t.id::text, to_jsonb(t) || {} FROM {schema}.\"{table}\" t",
        derived(schema, table)
    )
}

/// The workspace's schema (`"workspace_<id>"`, quoted). A CRM database has exactly one.
pub async fn schema(c: &Client) -> Result<String> {
    let row = c
        .query_one(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name LIKE 'workspace\\_%'",
            &[],
        )
        .await
        .context("finding the workspace's schema")?;
    Ok(format!("\"{}\"", row.get::<_, String>(0)))
}

/// Every record, by `<table>/<id>`.
pub async fn all(c: &Client) -> Result<BTreeMap<String, Value>> {
    let schema = schema(c).await?;
    let mut out = BTreeMap::new();
    for table in TABLES {
        let rows = c.query(&select(&schema, table), &[]).await?;
        for row in rows {
            let id: String = row.get(0);
            out.insert(format!("{table}/{id}"), row.get::<_, Value>(1));
        }
    }
    Ok(out)
}

/// Empties a new workspace of the demo records Twenty puts in it.
pub async fn purge_demo(c: &Client) -> Result<()> {
    let schema = schema(c).await?;
    let tables: Vec<String> = DEMO_TABLES
        .iter()
        .map(|t| format!("{schema}.\"{t}\""))
        .collect();
    c.batch_execute(&format!("TRUNCATE {} CASCADE", tables.join(", ")))
        .await
        .context("emptying the demo records")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_says_its_accounts_and_an_opportunity_its_account() {
        let note = select("\"w\"", "note");
        assert!(
            note.contains("'accounts'") && note.contains("\"w\".\"noteTarget\""),
            "{note}"
        );
        let deal = select("\"w\"", "opportunity");
        assert!(
            deal.contains("'account'") && deal.contains("t.\"companyId\""),
            "{deal}"
        );
        assert!(select("\"w\"", "company").contains("'{}'::jsonb"));
    }
}
