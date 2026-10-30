//! Gold databases: `world_base` with a task's gold actions applied.

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::{pg::Admin, schema::BASE_DB, tools};

/// Returns the name of `gold_<task_id>`, creating it from `world_base` if missing.
/// Built under a temporary name and renamed at the end, so a crashed earlier run never
/// leaves a half-built `gold_<id>`. A failing gold action leaves the database unchanged,
/// as upstream; at least one gold write must succeed.
pub async fn ensure_gold(admin: &Admin, task: &Value) -> Result<String> {
    let id = match &task["id"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(anyhow!("bad task id {id:?}"));
    }
    let name = format!("gold_{id}");
    if admin.exists(&name).await? {
        return Ok(name);
    }
    let tmp = format!("gold_tmp_{id}_{}", std::process::id());
    admin.drop_db(&tmp).await?;
    admin.create_db_from(&tmp, BASE_DB).await?;
    let actions = task["evaluation_criteria"]["actions"].as_array().cloned().unwrap_or_default();
    let built = admin
        .with_db(&tmp, async |client| {
            let mut wrote = false;
            for a in &actions {
                let name = a["name"].as_str().unwrap_or("");
                let out = tools::call(client, name, &a["arguments"]).await?;
                wrote = wrote || (out.error.is_none() && !out.writes.is_empty());
            }
            if !wrote {
                return Err(anyhow!("task {id}: no gold write succeeded"));
            }
            Ok(())
        })
        .await;
    let finished = match built {
        Ok(()) => admin.rename_db(&tmp, &name).await,
        Err(e) => Err(e),
    };
    if let Err(e) = finished {
        let _ = admin.drop_db(&tmp).await;
        // A concurrent creator may have won the rename.
        if admin.exists(&name).await.unwrap_or(false) {
            return Ok(name);
        }
        return Err(e);
    }
    Ok(name)
}
