//! Instances and saves: a database each (`ti_<id>`, `ts_<id>`), copied with the kit's Postgres
//! templates, and for a running instance a server and a record in `.live/<id>.json`.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use arena_part::helpers::pg::Templates;
use arena_part::proto::*;
use arena_part::PartError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::base::BASE_DB;
use crate::rows;
use crate::settings::Settings;
use crate::stack::{bring_up, free_port, take_down, ServerSpec, Stack};
use crate::{TwentyCrm, R};

/// A copy of the base started in 9 s on the machine this was written on.
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// Database names are at most 63 bytes, and `ti_` takes 3.
const MAX_ID: usize = 60;

fn instance_db(id: &str) -> String {
    format!("ti_{id}")
}

fn save_db(id: &str) -> String {
    format!("ts_{id}")
}

fn check_id(id: &str) -> R<()> {
    if valid_id(id) && id.len() <= MAX_ID {
        return Ok(());
    }
    Err(PartError::invalid(format!(
        "{id:?} is not an id: [a-z0-9_]{{1,{MAX_ID}}}"
    )))
}

fn internal(e: anyhow::Error) -> PartError {
    PartError::internal(format!("{e:#}"))
}

/// What a running instance needs after this program restarts.
#[derive(Debug, Serialize, Deserialize)]
struct Record {
    port: u16,
    project: String,
    db_port: u16,
}

fn live_dir(root: &Path) -> PathBuf {
    root.join(".live")
}

fn record_path(root: &Path, id: &str) -> PathBuf {
    live_dir(root).join(format!("{id}.json"))
}

fn read_record(root: &Path, id: &str) -> Option<Record> {
    let text = std::fs::read_to_string(record_path(root, id)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_record(root: &Path, id: &str, record: &Record) -> anyhow::Result<()> {
    std::fs::create_dir_all(live_dir(root))?;
    std::fs::write(record_path(root, id), serde_json::to_vec(record)?)
        .context("recording an instance")
}

fn live_ids(root: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(live_dir(root)).into_iter().flatten();
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".json").map(str::to_string)
        })
        .collect()
}

/// The port of a running instance's server; an unknown instance is -32002.
pub fn port_of(root: &Path, id: &str) -> R<u16> {
    read_record(root, id)
        .map(|r| r.port)
        .ok_or_else(|| PartError::unknown(format!("unknown instance {id}")))
}

/// The server a running instance is on, if one runs.
pub fn recorded_settings(root: &Path) -> Option<Settings> {
    let record = live_ids(root).iter().find_map(|id| read_record(root, id))?;
    let mut settings = Settings::from_env().ok()?;
    settings.project = record.project;
    settings.db_port = record.db_port;
    Some(settings)
}

fn kept(keep: &[String], id: &str) -> bool {
    keep.iter().any(|k| match k.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => k == id,
    })
}

impl TwentyCrm {
    /// The database to copy for a start: the save's, else the base.
    async fn source(&self, p: &StartParams) -> R<String> {
        let Some(from) = &p.from else {
            return Ok(BASE_DB.to_string());
        };
        if from.mode != Mode::Exact {
            return Err(PartError::cant_restore("twenty-crm restores exactly only"));
        }
        check_id(&from.save)?;
        let db = save_db(&from.save);
        let ready = self.ready(&p.config).await?;
        let exists = ready.stack.pg.exists(&db).await.map_err(internal)?;
        if !exists {
            return Err(PartError::cant_restore(format!(
                "unknown save {}",
                from.save
            )));
        }
        Ok(db)
    }

    pub(crate) async fn start_instance(&self, p: StartParams) -> R<()> {
        check_id(&p.instance)?;
        let ready = self.ready(&p.config).await?;
        if read_record(&self.root, &p.instance).is_some() {
            return Err(PartError::invalid(format!(
                "instance {} already runs",
                p.instance
            )));
        }
        let source = self.source(&p).await?;
        let db = instance_db(&p.instance);
        let pg = &ready.stack.pg;
        pg.drop(&db).await.map_err(internal)?;
        pg.create_from(&db, &source).await.map_err(internal)?;
        let started = self.start_server(&ready.stack, &p.instance, &db).await;
        if started.is_err() {
            let _ = pg.drop(&db).await;
        }
        started.map_err(internal)
    }

    /// The instance's server, answering, and its record; on failure it is taken down again.
    async fn start_server(&self, stack: &Stack, id: &str, db: &str) -> anyhow::Result<()> {
        let port = free_port()?;
        let spec = ServerSpec {
            instance: id,
            db,
            port,
            migrate: false,
        };
        let project = stack.server(&spec)?;
        if let Err(e) = bring_up(project.clone(), port, START_TIMEOUT).await {
            let _ = take_down(project).await;
            return Err(e);
        }
        let record = Record {
            port,
            project: stack.settings.project.clone(),
            db_port: stack.settings.db_port,
        };
        write_record(&self.root, id, &record)
    }

    pub(crate) async fn stop_instance(&self, id: &str) -> R<()> {
        check_id(id)?;
        let port = port_of(&self.root, id)?;
        let ready = self.ready(&Value::Null).await?;
        let spec = ServerSpec {
            instance: id,
            db: &instance_db(id),
            port,
            migrate: false,
        };
        let project = ready.stack.server(&spec).map_err(internal)?;
        take_down(project).await.map_err(internal)?;
        ready
            .stack
            .pg
            .drop(&instance_db(id))
            .await
            .map_err(internal)?;
        let _ = std::fs::remove_file(record_path(&self.root, id));
        Ok(())
    }

    /// Copies the instance's database. Twenty's connections to it are closed for the copy; its
    /// pools open new ones on the next request.
    pub(crate) async fn save_instance(&self, p: SaveParams) -> R<Saved> {
        check_id(&p.save)?;
        port_of(&self.root, &p.instance)?;
        let pg = &self.ready(&Value::Null).await?.stack.pg;
        let db = save_db(&p.save);
        pg.drop(&db).await.map_err(internal)?;
        pg.create_from_exclusive(&db, &instance_db(&p.instance))
            .await
            .map_err(internal)?;
        let bytes = pg.size(&db).await.map_err(internal)?.unwrap_or(0);
        Ok(Saved {
            bytes: bytes.max(0) as u64,
            exact: true,
        })
    }

    async fn target_db(&self, target: &Target) -> R<String> {
        let (db, what) = match target {
            Target::Instance { instance } => (instance_db(instance), instance),
            Target::Save { save } => (save_db(save), save),
        };
        check_id(what)?;
        let ready = self.ready(&Value::Null).await?;
        if ready.stack.pg.exists(&db).await.map_err(internal)? {
            return Ok(db);
        }
        Err(PartError::unknown(format!("unknown target {what}")))
    }

    async fn records_of(&self, target: &Target) -> R<std::collections::BTreeMap<String, Value>> {
        let db = self.target_db(target).await?;
        let pg = &self.ready(&Value::Null).await?.stack.pg;
        pg.with_db(&db, async |c| rows::all(c).await)
            .await
            .map_err(internal)
    }

    pub(crate) async fn inspect_target(&self, p: InspectParams) -> R<Docs> {
        let mut all = self.records_of(&p.target).await?;
        let docs = if p.keys.is_empty() {
            all.into_iter().collect()
        } else {
            let wanted = p.keys.into_iter().map(|k| {
                let doc = all.remove(&k).unwrap_or(Value::Null);
                (k, doc)
            });
            wanted.collect()
        };
        Ok(Docs { docs })
    }

    pub(crate) async fn diff_targets(&self, p: DiffParams) -> R<Differing> {
        // One database at a time: never two connections held at once.
        let a = self.records_of(&p.a).await?;
        let b = self.records_of(&p.b).await?;
        Ok(Differing::between(&a, &b))
    }

    pub(crate) async fn drop_save_db(&self, save: &str) -> R<()> {
        check_id(save)?;
        let pg = &self.ready(&Value::Null).await?.stack.pg;
        let db = save_db(save);
        if !pg.exists(&db).await.map_err(internal)? {
            return Err(PartError::unknown(format!("unknown save {save}")));
        }
        pg.drop(&db).await.map_err(internal)
    }

    /// The sizes of the instances' and saves' databases. Before anything has run there are none
    /// that this program knows of, and asking must not start a server.
    pub(crate) async fn bytes_used(&self) -> R<Usage> {
        let Some(ready) = self.ready.get() else {
            return Ok(Usage::default());
        };
        let mut bytes = 0i64;
        for prefix in ["ti_", "ts_"] {
            let list = ready.stack.pg.list(prefix).await.map_err(internal)?;
            bytes += list.iter().map(|(_, size)| size).sum::<i64>();
        }
        Ok(Usage {
            bytes: bytes.max(0) as u64,
        })
    }

    /// Drops the saves `keep` doesn't name and the databases of instances not running.
    pub(crate) async fn collect(&self, keep: &[String]) -> R<GcResult> {
        let pg = &self.ready(&Value::Null).await?.stack.pg;
        let live = live_ids(&self.root);
        let mut out = GcResult::default();
        let is_live = |id: &str| live.iter().any(|l| l == id);
        drop_unkept(pg, "ti_", is_live, &mut out)
            .await
            .map_err(internal)?;
        let is_kept = |id: &str| kept(keep, id);
        drop_unkept(pg, "ts_", is_kept, &mut out)
            .await
            .map_err(internal)?;
        Ok(out)
    }
}

/// Drops the databases named `<prefix><id>` whose id `keep` turns down.
async fn drop_unkept(
    pg: &Templates,
    prefix: &str,
    keep: impl Fn(&str) -> bool,
    out: &mut GcResult,
) -> anyhow::Result<()> {
    for (name, size) in pg.list(prefix).await? {
        if !keep(&name[prefix.len()..]) {
            pg.drop(&name).await?;
            out.dropped += 1;
            out.bytes += size.max(0) as u64;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_names_an_id_or_a_prefix() {
        let keep = ["base_*".to_string(), "s1".to_string()];
        assert!(kept(&keep, "base_factory") && kept(&keep, "s1"));
        assert!(!kept(&keep, "s2"));
    }

    #[test]
    fn ids_must_leave_room_for_the_database_prefix() {
        assert!(check_id("chk_a").is_ok());
        assert!(check_id(&"a".repeat(61)).is_err());
        assert!(check_id("Bad-Id").is_err());
    }

    #[test]
    fn a_record_finds_the_instances_port_and_server() {
        let root = std::env::temp_dir().join(format!("twenty-record-{}", std::process::id()));
        let record = Record {
            port: 7,
            project: "arena_twenty_x".into(),
            db_port: 8,
        };
        write_record(&root, "a", &record).unwrap();
        assert_eq!(port_of(&root, "a").unwrap(), 7);
        assert_eq!(recorded_settings(&root).unwrap().project, "arena_twenty_x");
        assert_eq!(port_of(&root, "b").unwrap_err().code, -32002);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
