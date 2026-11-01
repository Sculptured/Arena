//! Starting and stopping instances: the record first, then the server, its team and, for a
//! start from a save, the changes that waited in it proposed again. Stopping kills the recorded
//! processes (only while they are still the server on this instance's data folder) and removes
//! the folders.
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use arena_part::helpers::blocking;
use arena_part::proto::{FromSave, StartParams};
use arena_part::PartError;
use serde_json::Value;

use crate::api::Api;
use crate::config::Config;
use crate::live::Run;
use crate::saves::{self, Meta};
use crate::server::{self, Pids};
use crate::team::{self, PROPOSERS};
use crate::{reraise, Sculptured, R};

/// A save to start from: its folder, what it records and the changes it holds.
struct Restored {
    dir: PathBuf,
    meta: Meta,
    changes: Vec<Value>,
}

/// A start, ready to run.
struct Plan {
    run: Run,
    team: String,
    from: Option<Restored>,
}

impl Sculptured {
    pub(crate) async fn start_instance(&self, p: StartParams) -> R<()> {
        let id = &p.instance;
        let plan = self.plan(&p).await?;
        self.stop_instance(id).await?; // a leftover would keep its old data
        self.mine.lock().expect("mine lock").insert(id.clone());
        // recorded first, so a server that comes up is never unknown to a restarted program
        let recorded = self.live.record(id, &plan.run);
        let up = match recorded {
            Ok(()) => self.bring_up(id, plan.run, &plan.team, plan.from).await,
            Err(e) => Err(e.into()),
        };
        if let Err(e) = up {
            let _ = self.stop_instance(id).await;
            return Err(PartError::internal_from(e));
        }
        Ok(())
    }

    /// What a start needs before anything runs: its record (with the build of the binary and a
    /// new scratch folder), its team, and the save it starts from.
    async fn plan(&self, p: &StartParams) -> R<Plan> {
        let config = Config::parse(&p.config)?;
        let binary = std::path::absolute(&config.binary).map_err(PartError::internal_from)?;
        let b = binary.clone();
        let build = blocking(move || server::version(&b)).await;
        let build = build.map_err(PartError::internal_from)?;
        let from = match &p.from {
            Some(f) => Some(self.restored(f, &build)?),
            None => None,
        };
        let tmp = scratch().map_err(PartError::internal_from)?;
        let (config_value, pids) = (p.config.clone(), Pids::default());
        let run = Run {
            config: config_value,
            binary,
            build,
            port: None,
            tmp,
            pids,
        };
        Ok(Plan {
            run,
            team: config.team,
            from,
        })
    }

    /// The save `f` names, when this build can restore it and every change that waited in it
    /// can be proposed again (-32001 otherwise).
    fn restored(&self, f: &FromSave, build: &str) -> R<Restored> {
        let dir = self.saves.require(&f.save)?;
        let meta = saves::meta(&dir).map_err(PartError::internal_from)?;
        if meta.build != build {
            let made = &meta.build;
            let msg = format!("save {} was made by {made}; the binary is {build}", f.save);
            return Err(PartError::cant_restore(msg));
        }
        let changes = saves::changes(&dir).map_err(PartError::internal_from)?;
        let waiting = reraise::waiting(&changes).len();
        if waiting > PROPOSERS {
            let msg = format!(
                "save {} has {waiting} changes waiting for a decision; a restored instance can \
                 propose at most {PROPOSERS} again",
                f.save
            );
            return Err(PartError::cant_restore(msg));
        }
        Ok(Restored { dir, meta, changes })
    }

    /// Starts the server of `id` (on a copy of `from`'s data folder), its team, and proposes
    /// again what waited in `from`. Records the server's processes and port as they come.
    async fn bring_up(
        &self,
        id: &str,
        mut run: Run,
        team: &str,
        from: Option<Restored>,
    ) -> anyhow::Result<()> {
        let f = self.live.folders(id, &run);
        if let Some(r) = &from {
            let (dir, data) = (r.dir.clone(), f.data());
            blocking(move || saves::restore(&dir, &data)).await?;
        }
        let spawned = server::spawn(&run.binary, &f)?;
        run.pids = spawned.pids;
        self.live.record(id, &run)?;
        let ready = server::ready(&f, &spawned).await?;
        (run.port, run.pids.hostd) = (Some(ready.port), ready.hostd);
        self.live.record(id, &run)?;
        let api = Api::new(ready.port)?;
        team::ensure_running(&api, team).await?;
        if let Some(r) = from {
            let aliases = reraise::reraise(&api, team, &r.changes, r.meta.aliases).await?;
            self.live.set_aliases(id, &aliases)?;
        }
        Ok(())
    }

    /// Stops `id` when it runs (a no-op otherwise).
    pub(crate) async fn stop_instance(&self, id: &str) -> R<()> {
        if let Some(run) = self.live.run(id) {
            self.bring_down(id, &run).await?;
        }
        self.live.remove(id, None); // a folder left without a record
        self.mine.lock().expect("mine lock").remove(id);
        self.busy.lock().expect("busy map lock").remove(id);
        Ok(())
    }

    /// Kills the server of `id` and removes its folders.
    async fn bring_down(&self, id: &str, run: &Run) -> R<()> {
        let f = self.live.folders(id, run);
        let killed = server::kill(run.binary.clone(), f.data(), run.pids).await;
        killed.map_err(PartError::internal_from)?;
        self.live.remove(id, Some(&f.tmp));
        Ok(())
    }

    /// Stops every instance with a folder here that this value did not start, going on after a
    /// failure; the failures are reported together at the end.
    pub(crate) async fn stop_leaked(&self) -> R<()> {
        let leaked: Vec<String> = {
            let mine = self.mine.lock().expect("mine lock");
            self.live
                .ids()
                .into_iter()
                .filter(|id| !mine.contains(id))
                .collect()
        };
        stop_each(leaked, |id| async move { self.stop_instance(&id).await }).await
    }
}

/// Runs `stop` on each of `ids`, going on after a failure; every failure, named, in one error.
async fn stop_each<F, Fut>(ids: Vec<String>, stop: F) -> R<()>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = R<()>>,
{
    let mut failed = vec![];
    for id in ids {
        if let Err(e) = stop(id.clone()).await {
            failed.push(format!("{id}: {}", e.message));
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    Err(PartError::internal(format!(
        "could not stop {}",
        failed.join("; ")
    )))
}

/// A new scratch folder of its own (`<temp>/arena-sc-<16 hex>`), short enough for the runner's
/// socket path (at most 108 bytes, so it can't be under the saves).
fn scratch() -> io::Result<PathBuf> {
    let seed = RandomState::new();
    for n in 0u32..100 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("arena-sc-{:016x}", seed.hash_one((nanos, n))));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free scratch folder"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_failing_stop_does_not_stop_the_others() {
        let ids: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let stopped = std::sync::Mutex::new(vec![]);
        let r = stop_each(ids.clone(), |id| {
            stopped.lock().unwrap().push(id.clone());
            async move {
                match id.as_str() {
                    "b" | "d" => Err(PartError::internal(format!("{id} is stuck"))),
                    _ => Ok(()),
                }
            }
        })
        .await;
        assert_eq!(stopped.into_inner().unwrap(), ids);
        let e = r.unwrap_err();
        assert_eq!(e.message, "could not stop b: b is stuck; d: d is stuck");
        assert_eq!(stop_each(vec![], |_| async { Ok(()) }).await, Ok(()));
    }
}
