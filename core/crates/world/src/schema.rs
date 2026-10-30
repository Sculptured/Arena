use std::path::Path;

use anyhow::{Context, Result};
use tokio_postgres::Client;

use crate::pg::Admin;

pub const BASE_DB: &str = "world_base";
pub const TABLES: [&str; 3] = ["users", "orders", "products"];

/// Creates `world_base` if missing and loads `db.json` into it. Idempotent.
pub async fn load_base(admin: &Admin, data_dir: &Path) -> Result<()> {
    if !admin.exists(BASE_DB).await? {
        admin.create_db(BASE_DB).await?;
    }
    let path = data_dir.join("db.json");
    let db: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).with_context(|| path.display().to_string())?)?;

    admin
        .with_db(BASE_DB, async |client| {
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
