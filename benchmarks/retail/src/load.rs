use std::path::Path;

use anyhow::{Context, Result};
use tokio_postgres::{Client, NoTls};

pub const TABLES: [&str; 3] = ["users", "orders", "products"];

/// Runs `f` on a fresh connection to `url`, then closes it for real (drops the client and
/// awaits the connection task) before returning, so no backend outlives the call.
pub async fn with_conn<T>(url: &str, f: impl AsyncFnOnce(&mut Client) -> Result<T>) -> Result<T> {
    let (mut client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .with_context(|| "connecting to the retail database")?;
    let handle = tokio::spawn(conn);
    let out = f(&mut client).await;
    drop(client);
    let _ = handle.await;
    out
}

/// Creates the three tables and loads `db.json` into the database at `url`. Idempotent.
pub async fn seed(url: &str, data_dir: &Path) -> Result<()> {
    let path = data_dir.join("db.json");
    let db: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).with_context(|| path.display().to_string())?,
    )?;
    with_conn(url, async |client| {
        let tx = client.transaction().await?;
        for t in TABLES {
            tx.batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS {t} (id text PRIMARY KEY, doc jsonb NOT NULL)"
            ))
            .await?;
            let docs = db.get(t).with_context(|| format!("db.json missing `{t}`"))?;
            tx.execute(
                &format!("INSERT INTO {t} (id, doc) SELECT key, value FROM jsonb_each($1::jsonb) ON CONFLICT (id) DO UPDATE SET doc = EXCLUDED.doc"),
                &[docs],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    })
    .await
}

/// Canonical dump: `{"users":{id:doc},"orders":{..},"products":{..}}`.
pub async fn dump(client: &Client) -> Result<serde_json::Value> {
    let mut out = serde_json::Map::new();
    for t in TABLES {
        let row = client
            .query_one(
                &format!("SELECT COALESCE(jsonb_object_agg(id, doc), '{{}}'::jsonb) FROM {t}"),
                &[],
            )
            .await?;
        out.insert(t.to_string(), row.get(0));
    }
    Ok(serde_json::Value::Object(out))
}
