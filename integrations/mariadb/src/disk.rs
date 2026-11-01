//! `usage` and `gc`: the binlog and the databases on the servers, and the saves on disk.
use std::collections::HashMap;

use arena_part::helpers::folder;
use arena_part::helpers::mariadb::{purge_target, Manifest, Position, Server};
use arena_part::proto::{kept, valid_id, GcResult, Usage};
use arena_part::PartError;

use crate::views::VIEW_PREFIX;
use crate::Mariadb;

const INSTANCE_PREFIX: &str = "mi_";

impl Mariadb {
    /// The servers this program used plus those the saves on disk name, that answer now (a
    /// server that is down holds nothing this program could free).
    async fn reachable_servers(&self, saves: &[(String, Manifest)]) -> Vec<Server> {
        let mut all: HashMap<String, Server> = HashMap::new();
        for s in self.servers.known() {
            all.insert(s.config().container.clone(), s);
        }
        for (_, m) in saves {
            if let (false, Ok(s)) = (
                all.contains_key(&m.server.container),
                self.servers.for_ref(&m.server),
            ) {
                all.insert(m.server.container.clone(), s);
            }
        }
        let mut out = vec![];
        for s in all.into_values() {
            if let Ok(c) = s.conn(None).await {
                let _ = c.disconnect().await;
                out.push(s);
            }
        }
        out
    }

    pub(crate) async fn bytes_used(&self) -> Result<Usage, PartError> {
        let mut bytes = 0;
        for s in self.reachable_servers(&self.manifests()).await {
            bytes += s.binlog_files().await?.iter().map(|f| f.size).sum::<u64>();
            for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
                bytes += s.databases(prefix).await?.iter().map(|d| d.1).sum::<u64>();
            }
        }
        Ok(Usage { bytes })
    }

    pub(crate) async fn collect(&self, keep: &[String]) -> Result<GcResult, PartError> {
        let mut out = self.drop_saves_not_kept(keep);
        let saves = self.manifests();
        for s in self.reachable_servers(&saves).await {
            let (n, bytes) = self.drop_idle_databases(&s).await?;
            out.dropped += n;
            out.bytes += bytes;
            s.purge_to(purge_target(&self.needed(&s, &saves), &s.position().await?))
                .await?;
        }
        out.bytes += self.remove_unused_blobs().await?;
        Ok(out)
    }

    fn drop_saves_not_kept(&self, keep: &[String]) -> GcResult {
        let mut out = GcResult::default();
        let Ok(rd) = std::fs::read_dir(self.dirs.root()) else {
            return out;
        };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_id(&name) && !kept(keep, &name) && e.path().is_dir() {
                let bytes = folder::size(&e.path());
                if self.dirs.remove(&name).is_ok() {
                    out.dropped += 1;
                    out.bytes += bytes;
                }
            }
        }
        out
    }

    /// Drops the instance databases that are not live and the scratch ones nobody reads.
    async fn drop_idle_databases(&self, s: &Server) -> Result<(u64, u64), PartError> {
        let mut busy = self.open_views();
        busy.extend(self.live_on(s).into_iter().map(|(db, _, _)| db));
        let (mut n, mut bytes) = (0, 0);
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
            for (db, size) in s.databases(prefix).await? {
                if !busy.contains(&db) {
                    s.drop_db(&db).await?;
                    n += 1;
                    bytes += size;
                }
            }
        }
        Ok((n, bytes))
    }

    /// The live instances on `s`: database, base manifest and start.
    fn live_on(&self, s: &Server) -> Vec<(String, Manifest, Position)> {
        let live = self.live.lock().expect("instances lock");
        live.values()
            .filter(|i| i.server.reference() == s.reference())
            .map(|i| (i.db.clone(), i.base.clone(), i.start.clone()))
            .collect()
    }

    /// The binlog positions on `s` that a kept save or a live instance still needs.
    fn needed(&self, s: &Server, saves: &[(String, Manifest)]) -> Vec<Position> {
        let mine = |m: &&Manifest| m.server == s.reference();
        let mut out: Vec<Position> = saves
            .iter()
            .map(|(_, m)| m)
            .filter(mine)
            .filter_map(|m| m.oldest().cloned())
            .collect();
        for (_, base, start) in self.live_on(s) {
            out.extend(base.oldest().cloned());
            out.push(start);
        }
        out
    }
}
