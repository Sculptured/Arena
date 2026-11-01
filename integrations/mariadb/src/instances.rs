//! Live instances: start (fresh or from a save), tool calls and stop.
use arena_part::helpers::mariadb::{Manifest, Position, Purged, Server};
use arena_part::proto::{CallToolParams, StartParams, ToolOutput};
use arena_part::PartError;
use serde_json::Value;

use crate::settings::Settings;
use crate::{keys, rows, sql, Mariadb};

/// An instance is the database `mi_<id>`. Its saves are its `base` (what it started from)
/// followed by its own changes, the binlog from `start` on.
#[derive(Debug, Clone)]
pub struct Instance {
    pub server: Server,
    pub db: String,
    pub base: Manifest,
    pub start: Position,
    pub dump_after: u64,
}

/// The database of instance `id`. MariaDB names are at most 64 characters.
pub fn instance_db(id: &str) -> Result<String, PartError> {
    let db = format!("mi_{id}");
    if db.len() > 64 {
        return Err(PartError::invalid(
            "instance ids for mariadb are at most 61 characters",
        ));
    }
    Ok(db)
}

/// A failed restore: -32001 when the binlog it needs was purged.
fn restore_error(save: &str, e: anyhow::Error) -> PartError {
    match e.downcast_ref::<Purged>() {
        Some(p) => PartError::cant_restore(format!("can't restore {save}: {p}")),
        None => PartError::internal(format!("restoring {save}: {e:#}")),
    }
}

fn tool_error(m: String) -> ToolOutput {
    ToolOutput {
        error: Some(m),
        ..Default::default()
    }
}

impl Mariadb {
    pub(crate) fn instance(&self, id: &str) -> Result<Instance, PartError> {
        let live = self.live.lock().expect("instances lock");
        let found = live.get(id).cloned();
        found.ok_or_else(|| PartError::unknown(format!("unknown instance {id}")))
    }

    pub(crate) async fn start_instance(&self, p: StartParams) -> Result<(), PartError> {
        let settings = Settings::from_config(&p.config)?;
        let db = instance_db(&p.instance)?;
        let server = self.servers.ensure(settings.server.clone()).await?;
        let (base, start) = match &p.from {
            Some(f) => self.restored(&server, &db, &f.save).await?,
            None => fresh(&server, &db, settings.init_sql.as_deref()).await?,
        };
        let inst = Instance {
            server,
            db,
            base,
            start,
            dump_after: settings.dump_after_bytes,
        };
        self.live
            .lock()
            .expect("instances lock")
            .insert(p.instance, inst);
        Ok(())
    }

    /// `db` rebuilt from the save; its own changes start after the replayed history.
    async fn restored(
        &self,
        server: &Server,
        db: &str,
        save: &str,
    ) -> Result<(Manifest, Position), PartError> {
        let m = self.manifest(save)?;
        if m.server != server.reference() {
            return Err(PartError::cant_restore(format!(
                "save {save} was made on {}, this instance is set for {}",
                m.server.container,
                server.config().container
            )));
        }
        let start = server.restore(&m, &self.blobs, db).await;
        let start = start.map_err(|e| restore_error(save, e))?;
        Ok((m, start))
    }

    pub(crate) async fn stop_instance(&self, id: &str) -> Result<(), PartError> {
        let inst = self.live.lock().expect("instances lock").remove(id);
        match inst {
            Some(i) => Ok(i.server.drop_db(&i.db).await?),
            None => Ok(()),
        }
    }

    pub(crate) async fn tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let inst = self.instance(&p.instance)?;
        let read = match (p.name.as_str(), self.sql_tools) {
            ("sql_query", true) => true,
            ("sql_execute", true) => false,
            (other, _) => return Ok(tool_error(format!("unknown tool {other}"))),
        };
        let (text, params) = match sql::args(&p.args) {
            Ok(a) => a,
            Err(m) => return Ok(tool_error(m)),
        };
        let ran = inst
            .server
            .with_db(&inst.db, async |c| {
                let pks = rows::pk_columns(c).await?;
                let out = if read {
                    sql::query(c, text, params).await
                } else {
                    sql::execute(c, text, params).await
                };
                Ok((out, keys::keys(text, params, &pks)))
            })
            .await?;
        Ok(tool_output(read, ran))
    }
}

/// A tool's result with the keys it touched, as reads or as writes.
fn tool_output(read: bool, (out, touched): (Result<Value, String>, Vec<String>)) -> ToolOutput {
    let result = match out {
        Ok(v) => Some(v),
        Err(e) => return tool_error(e),
    };
    let (reads, writes) = if read {
        (touched, vec![])
    } else {
        (vec![], touched)
    };
    ToolOutput {
        result,
        reads,
        writes,
        error: None,
    }
}

/// A new empty `db`, then `init_sql` in it (logged: it is part of the instance's own changes).
async fn fresh(
    server: &Server,
    db: &str,
    init_sql: Option<&str>,
) -> Result<(Manifest, Position), PartError> {
    server.create_db(db).await?;
    let start = server.position().await?;
    if let Some(text) = init_sql {
        let ran = server
            .with_db(db, async |c| Ok(sql::execute(c, text, &[]).await))
            .await?;
        if let Err(e) = ran {
            let _ = server.drop_db(db).await;
            return Err(PartError::invalid(format!("init_sql failed: {e}")));
        }
    }
    Ok((Manifest::empty(server.reference()), start))
}
