//! Live instances: start (fresh or from a save), tool calls and stop.
//!
//! Each instance has its own account, named like its database, that may use only that database:
//! its tools (and `connect`) can't turn binlogging off, purge the binlog or reach another database.
use arena_part::helpers::mariadb::{Login, Manifest, Position, Purged, Server, ServerRef};
use arena_part::helpers::sql_tools::{sql_args, tool_error, tool_output, which, SqlTool};
use arena_part::proto::{CallToolParams, StartParams, ToolOutput};
use arena_part::PartError;

use crate::settings::Settings;
use crate::{keys, rows, sql, Mariadb};

/// An instance is the database `mi_<id>`. Its saves are its `base` (what it started from)
/// followed by its own changes, the binlog from `start` on.
#[derive(Debug, Clone)]
pub struct Instance {
    pub server: Server,
    pub server_ref: ServerRef,
    pub db: String,
    pub login: Login,
    pub base: Manifest,
    pub start: Position,
    /// Up to here the instance's changes are known to be all in its own segment (see
    /// `Mariadb::check_segment`).
    pub checked: Position,
    pub dump_after: u64,
}

/// The database (and account) of instance `id`. MariaDB names are at most 64 characters.
pub fn instance_db(id: &str) -> Result<String, PartError> {
    let db = format!("mi_{id}");
    if db.len() > 64 {
        return Err(PartError::invalid(
            "instance ids for mariadb are at most 61 characters",
        ));
    }
    Ok(db)
}

/// -32001 unless `save` was made on the server `here` (same container and identity).
pub fn check_same_server(save: &str, saved: &ServerRef, here: &ServerRef) -> Result<(), PartError> {
    if saved.container != here.container || saved.port != here.port {
        return Err(PartError::cant_restore(format!(
            "save {save} was made on {}, this instance is set for {}",
            saved.container, here.container
        )));
    }
    if saved.id != here.id {
        return Err(PartError::cant_restore(format!(
            "can't restore {save}: the MariaDB server was replaced since this save"
        )));
    }
    Ok(())
}

/// A failed restore: -32001 when the binlog it needs was purged.
pub fn restore_error(save: &str, e: anyhow::Error) -> PartError {
    match e.downcast_ref::<Purged>() {
        Some(p) => PartError::cant_restore(format!("can't restore {save}: {p}")),
        None => PartError::internal(format!("restoring {save}: {e:#}")),
    }
}

/// What a start builds before the instance is live.
struct Built {
    base: Manifest,
    start: Position,
    login: Login,
}

impl Built {
    fn into_instance(
        self,
        server: Server,
        server_ref: ServerRef,
        db: String,
        dump_after: u64,
    ) -> Instance {
        Instance {
            server,
            server_ref,
            db,
            login: self.login,
            base: self.base,
            checked: self.start.clone(),
            start: self.start,
            dump_after,
        }
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
        let server_ref = server.reference().await?;
        let _busy = self.busy.mark(&db);
        let built = self.build(&server, &server_ref, &db, &p, &settings).await;
        let built = match built {
            Ok(b) => b,
            Err(e) => {
                let _ = server.drop_login(&db).await;
                let _ = server.drop_db(&db).await;
                return Err(e);
            }
        };
        let inst = built.into_instance(server, server_ref, db, settings.dump_after_bytes);
        self.live
            .lock()
            .expect("instances lock")
            .insert(p.instance, inst);
        Ok(())
    }

    async fn build(
        &self,
        server: &Server,
        here: &ServerRef,
        db: &str,
        p: &StartParams,
        settings: &Settings,
    ) -> Result<Built, PartError> {
        let Some(from) = &p.from else {
            return fresh(server, here, db, settings.init_sql.as_deref()).await;
        };
        let m = self.manifest(&from.save)?;
        check_same_server(&from.save, &m.server, here)?;
        let start = server.restore(&m, &self.blobs, db).await;
        let start = start.map_err(|e| restore_error(&from.save, e))?;
        let login = server.create_login(db, db, "").await?;
        Ok(Built {
            base: m,
            start,
            login,
        })
    }

    pub(crate) async fn stop_instance(&self, id: &str) -> Result<(), PartError> {
        let inst = self.live.lock().expect("instances lock").remove(id);
        let Some(i) = inst else {
            return Ok(());
        };
        // Both, even when one fails; the first error is the answer.
        let login = i.server.drop_login(&i.login.user).await;
        let db = i.server.drop_db(&i.db).await;
        login.and(db).map_err(PartError::from)
    }

    pub(crate) async fn tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let inst = self.instance(&p.instance)?;
        let tool = match which(&p.name, self.sql_tools) {
            Ok(t) => t,
            Err(out) => return Ok(out),
        };
        let (text, params) = match sql_args(&p.args) {
            Ok(a) => a,
            Err(m) => return Ok(tool_error(m)),
        };
        let (out, touched) = inst
            .server
            .with_login(&inst.login, Some(&inst.db), async |c| {
                let pks = rows::pk_columns(c).await?;
                let out = match tool {
                    SqlTool::Query => sql::query(c, text, params).await,
                    SqlTool::Execute => sql::execute(c, text, params).await,
                };
                Ok((out, keys::keys(text, params, &pks)))
            })
            .await?;
        Ok(tool_output(tool, out, touched))
    }
}

/// A new empty `db` and its account, then `init_sql` run by that account (logged: it is part of
/// the instance's own changes).
async fn fresh(
    server: &Server,
    here: &ServerRef,
    db: &str,
    init_sql: Option<&str>,
) -> Result<Built, PartError> {
    server.create_db(db).await?;
    let login = server.create_login(db, db, "").await?;
    let start = server.position().await?;
    if let Some(text) = init_sql {
        let ran = server
            .with_login(&login, Some(db), async |c| {
                Ok(sql::execute(c, text, &[]).await)
            })
            .await?;
        ran.map_err(|e| PartError::invalid(format!("init_sql failed: {e}")))?;
    }
    Ok(Built {
        base: Manifest::empty(here.clone()),
        start,
        login,
    })
}
