//! `usage`, `drop_save` and `gc`: the binlog, databases, accounts and sites, and the saves and
//! dumps on disk.
use std::collections::HashSet;

use arena_part::helpers::mariadb::{replay_user, Position, Server};
use arena_part::proto::{valid_id, GcResult, Usage};
use arena_part::PartError;

use crate::names::{INSTANCE_PREFIX, VIEW_PREFIX};
use crate::Erpnext;

impl Erpnext {
    /// What this part keeps outside `ARENA_SAVES`: its server's binlog and the sites'
    /// databases. (The sites' folders are under `ARENA_SAVES/erpnext/.sites`, which arenad
    /// counts.) Nothing when the server is not running.
    pub(crate) async fn bytes_used(&self) -> Result<Usage, PartError> {
        let server = self.server();
        if !server.reachable().await {
            return Ok(Usage { bytes: 0 });
        }
        let mut bytes: u64 = server.binlog_files().await?.iter().map(|f| f.size).sum();
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
            bytes += server
                .databases(prefix)
                .await?
                .iter()
                .map(|d| d.1)
                .sum::<u64>();
        }
        Ok(Usage { bytes })
    }

    /// Removes the save's folder; tidying up after it (dumps, binlog) is best effort, since the
    /// save itself is gone either way.
    pub(crate) async fn drop_save_files(&self, save: &str) -> Result<(), PartError> {
        self.dirs.remove(save).map_err(PartError::internal_from)?;
        if let Err(e) = self.settle_on(self.server()).await {
            eprintln!("arena-erpnext: dropped save {save}, but tidying up failed: {e}");
        }
        Ok(())
    }

    pub(crate) async fn collect(&self, keep: &[String]) -> Result<GcResult, PartError> {
        let mut out = self
            .dirs
            .remove_not_kept(keep)
            .map_err(PartError::internal_from)?;
        if self.server().reachable().await {
            let idle = self.drop_idle().await?;
            out.dropped += idle.dropped;
            out.bytes += idle.bytes;
        }
        out.bytes += self.settle_on(self.server()).await?;
        Ok(out)
    }

    /// After saves went: removes the dumps nothing needs and purges the binlog before the
    /// oldest position a save or a live instance still needs. When nothing needs any and no
    /// site is being made, the binlog is rotated first, so all of it goes. Returns the dump
    /// bytes freed.
    pub(crate) async fn settle_on(&self, server: &Server) -> Result<u64, PartError> {
        let freed = self.remove_unused_blobs().await?;
        if !server.reachable().await {
            return Ok(freed);
        }
        let needed = self.needed(server).await?;
        let alone = match needed.is_empty() {
            true => self.starts.try_write().ok(),
            false => None,
        };
        if alone.is_some() {
            server.rotate().await?;
        }
        server.purge_unneeded(&needed).await?;
        Ok(freed)
    }

    /// The binlog positions on this server that a save or a live instance still needs.
    async fn needed(&self, server: &Server) -> Result<Vec<Position>, PartError> {
        let here = server.reference().await?;
        let mut out: Vec<_> = self
            .manifests()?
            .into_iter()
            .filter(|(_, m)| m.server == here)
            .filter_map(|(_, m)| m.oldest().cloned())
            .collect();
        let live = self.live.lock().expect("instances lock");
        out.extend(live.values().flat_map(|i| i.tracked.needed().cloned()));
        Ok(out)
    }

    /// The sites in use: live instances' and scratch sites being read.
    fn busy_sites(&self) -> (HashSet<String>, HashSet<String>) {
        let mut dbs = self.busy.snapshot();
        let live = self.live.lock().expect("instances lock");
        dbs.extend(live.values().map(|i| i.site.db.clone()));
        let names = live.values().map(|i| i.site.name.clone()).collect();
        (dbs, names)
    }

    /// Drops the databases (counted), accounts (and replay accounts a crash left) and site
    /// folders of sites not in use.
    async fn drop_idle(&self) -> Result<GcResult, PartError> {
        let (busy_dbs, live_sites) = self.busy_sites();
        let mut out = GcResult::default();
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
            for (db, size) in self.server().databases(prefix).await? {
                if !busy_dbs.contains(&db) {
                    let server = self.server();
                    server.drop_db(&db).await?;
                    server.drop_login(&db).await?;
                    server.drop_login(&replay_user(&db)).await?;
                    out.dropped += 1;
                    out.bytes += size;
                }
            }
        }
        self.drop_idle_folders(&live_sites, &busy_dbs);
        Ok(out)
    }

    /// Removes the folders of instance sites not live and scratch sites not being read.
    fn drop_idle_folders(&self, live_sites: &HashSet<String>, busy_dbs: &HashSet<String>) {
        let sites = crate::sites::SitesDir::new(self.dirs.root().join(".sites"));
        for name in sites.names() {
            let instance = name.strip_suffix(".localhost").is_some_and(valid_id);
            let idle_instance = instance && !live_sites.contains(&name);
            let idle_view = view_db(&name).is_some_and(|db| !busy_dbs.contains(&db));
            if idle_instance || idle_view {
                let _ = sites.remove(&name);
            }
        }
    }
}

/// The database of a scratch site name (`view-<pid>-<n>.localhost`).
fn view_db(name: &str) -> Option<String> {
    let rest = name.strip_prefix("view-")?.strip_suffix(".localhost")?;
    let (pid, n) = rest.split_once('-')?;
    let (pid, n) = (pid.parse::<u32>().ok()?, n.parse::<u64>().ok()?);
    Some(format!("{VIEW_PREFIX}{pid}_{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::Site;

    #[test]
    fn scratch_site_folders_name_their_database() {
        let v = Site::view(7);
        assert_eq!(view_db(&v.name), Some(v.db));
        assert_eq!(view_db("i1.localhost"), None);
        assert_eq!(view_db("view-x-1.localhost"), None);
    }
}
