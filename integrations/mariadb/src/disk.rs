//! `usage`, `drop_save` and `gc`: the binlog, databases and accounts on the servers, and the
//! saves and dumps on disk.
use std::collections::{HashMap, HashSet};

use arena_part::helpers::folder;
use arena_part::helpers::mariadb::{purge_target, Manifest, Position, Server, ServerRef};
use arena_part::proto::{kept, valid_id, GcResult, Usage};
use arena_part::PartError;

use crate::views::VIEW_PREFIX;
use crate::Mariadb;

const INSTANCE_PREFIX: &str = "mi_";
const REPLAY_PREFIX: &str = "rp_";

impl Mariadb {
    /// The servers this program used plus those the saves on disk name, that answer now (a
    /// server that is down holds nothing this program could free).
    async fn reachable_servers(&self, saves: &[(String, Manifest)]) -> Vec<Server> {
        let mut all: HashMap<String, Server> = HashMap::new();
        for s in self.servers.known() {
            all.insert(s.config().container.clone(), s);
        }
        for (_, m) in saves {
            if all.contains_key(&m.server.container) {
                continue;
            }
            if let Ok(s) = self.servers.for_ref(&m.server) {
                all.insert(m.server.container.clone(), s);
            }
        }
        let mut out = vec![];
        for s in all.into_values() {
            if s.reachable().await {
                out.push(s);
            }
        }
        out
    }

    pub(crate) async fn bytes_used(&self) -> Result<Usage, PartError> {
        let mut bytes = 0;
        for s in self.reachable_servers(&self.manifests()?).await {
            bytes += s.binlog_files().await?.iter().map(|f| f.size).sum::<u64>();
            for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
                bytes += s.databases(prefix).await?.iter().map(|d| d.1).sum::<u64>();
            }
        }
        Ok(Usage { bytes })
    }

    /// Removes the save's folder; tidying up after it (dumps, binlog) is best effort, since the
    /// save itself is gone either way.
    pub(crate) async fn drop_save_files(&self, save: &str) -> Result<(), PartError> {
        self.dirs.remove(save).map_err(PartError::internal_from)?;
        if let Err(e) = self.settle().await {
            eprintln!("arena-mariadb: dropped save {save}, but tidying up failed: {e}");
        }
        Ok(())
    }

    pub(crate) async fn collect(&self, keep: &[String]) -> Result<GcResult, PartError> {
        let mut out = self.drop_saves_not_kept(keep)?;
        for s in self.reachable_servers(&self.manifests()?).await {
            let idle = self.drop_idle(&s).await?;
            out.dropped += idle.dropped;
            out.bytes += idle.bytes;
        }
        out.bytes += self.settle().await?;
        Ok(out)
    }

    /// After saves went: removes the dumps nothing needs and purges the binlog files before the
    /// oldest position a save or a live instance still needs. Returns the dump bytes freed.
    async fn settle(&self) -> Result<u64, PartError> {
        let freed = self.remove_unused_blobs().await?;
        let saves = self.manifests()?;
        for s in self.reachable_servers(&saves).await {
            // One server that misbehaves must not stop the others being purged.
            if let Err(e) = self.purge(&s, &saves).await {
                let name = &s.config().container;
                eprintln!("arena-mariadb: not purging {name}: {e}");
            }
        }
        Ok(freed)
    }

    /// Purges the binlog files on `s` before the oldest position still needed there.
    async fn purge(&self, s: &Server, saves: &[(String, Manifest)]) -> Result<(), PartError> {
        let here = s.reference().await?;
        let needed = self.needed(&here, saves);
        let (files, now) = (s.binlog_files().await?, s.position().await?);
        if let Some(file) = purge_target(&needed, &now, &files) {
            s.purge_to(file).await?;
        }
        Ok(())
    }

    fn drop_saves_not_kept(&self, keep: &[String]) -> Result<GcResult, PartError> {
        let mut out = GcResult::default();
        let Ok(rd) = std::fs::read_dir(self.dirs.root()) else {
            return Ok(out);
        };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_id(&name) && !kept(keep, &name) && e.path().is_dir() {
                let bytes = folder::size(&e.path());
                self.dirs
                    .remove(&name)
                    .map_err(|err| PartError::internal(format!("removing save {name}: {err}")))?;
                out.dropped += 1;
                out.bytes += bytes;
            }
        }
        Ok(out)
    }

    /// The databases nobody uses: instances not running, scratch copies nobody reads, and the
    /// accounts of either.
    fn busy_dbs(&self) -> HashSet<String> {
        let mut busy = self.busy.snapshot();
        let live = self.live.lock().expect("instances lock");
        busy.extend(live.values().map(|i| i.db.clone()));
        busy
    }

    /// Drops idle databases (counted) and idle accounts on `s`.
    async fn drop_idle(&self, s: &Server) -> Result<GcResult, PartError> {
        let busy = self.busy_dbs();
        let mut out = GcResult::default();
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
            for (db, size) in s.databases(prefix).await? {
                if !busy.contains(&db) {
                    s.drop_db(&db).await?;
                    out.dropped += 1;
                    out.bytes += size;
                }
            }
        }
        for prefix in [INSTANCE_PREFIX, REPLAY_PREFIX] {
            for user in s.logins(prefix).await? {
                let db = user.strip_prefix(REPLAY_PREFIX).unwrap_or(&user);
                if !busy.contains(db) {
                    s.drop_login(&user).await?;
                }
            }
        }
        Ok(out)
    }

    /// The binlog positions on the server `here` that a save or a live instance still needs.
    fn needed(&self, here: &ServerRef, saves: &[(String, Manifest)]) -> Vec<Position> {
        let mut out: Vec<Position> = saves
            .iter()
            .filter(|(_, m)| &m.server == here)
            .filter_map(|(_, m)| m.oldest().cloned())
            .collect();
        let live = self.live.lock().expect("instances lock");
        for i in live.values().filter(|i| &i.server_ref == here) {
            out.extend(i.base.oldest().cloned());
            out.push(i.start.clone());
        }
        out
    }
}
