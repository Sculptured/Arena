//! `usage`, `drop_save` and `gc`: the binlog, databases, accounts and sites, and the saves and
//! dumps on disk.
use std::collections::HashSet;

use arena_part::helpers::mariadb::{needed_positions, replay_user, Position, Server};
use arena_part::proto::{valid_id, GcResult, Usage};
use arena_part::PartError;

use crate::names::{Site, INSTANCE_PREFIX, VIEW_PREFIX};
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
        let saves = self.manifests()?;
        let live = self.live.lock().expect("instances lock");
        Ok(needed_positions(
            &here,
            &saves,
            live.values().map(|i| &i.tracked),
        ))
    }

    /// The databases in use: live instances', starting ones' and scratch sites being read.
    fn busy_dbs(&self) -> HashSet<String> {
        let mut dbs = self.busy.snapshot();
        let live = self.live.lock().expect("instances lock");
        dbs.extend(live.values().map(|i| i.site.db.clone()));
        dbs
    }

    /// Drops the databases (counted), accounts (and replay accounts a crash left) and site
    /// folders of sites not in use.
    async fn drop_idle(&self) -> Result<GcResult, PartError> {
        let busy = self.busy_dbs();
        let server = self.server();
        let mut out = GcResult::default();
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX] {
            for (db, size) in server.databases(prefix).await? {
                if !busy.contains(&db) {
                    server.drop_db(&db).await?;
                    out.dropped += 1;
                    out.bytes += size;
                }
            }
        }
        self.drop_idle_logins(&busy).await?;
        self.drop_idle_folders(&busy);
        Ok(out)
    }

    /// Drops the site accounts, and the replay accounts, of databases not in use.
    async fn drop_idle_logins(&self, busy: &HashSet<String>) -> Result<(), PartError> {
        let server = self.server();
        let replay = replay_user("");
        for prefix in [INSTANCE_PREFIX, VIEW_PREFIX, replay.as_str()] {
            for user in server.logins(prefix).await? {
                let db = user.strip_prefix(replay.as_str()).unwrap_or(&user);
                let ours = db.starts_with(INSTANCE_PREFIX) || db.starts_with(VIEW_PREFIX);
                if ours && !busy.contains(db) {
                    server.drop_login(&user).await?;
                }
            }
        }
        Ok(())
    }

    /// Removes the folders of sites whose database is not in use.
    fn drop_idle_folders(&self, busy: &HashSet<String>) {
        let sites = crate::sites::SitesDir::new(self.dirs.root().join(".sites"));
        for name in sites.names() {
            if site_db(&name).is_some_and(|db| !busy.contains(&db)) {
                let _ = sites.remove(&name);
            }
        }
    }
}

/// The database of a site folder this part makes: an instance's or a scratch site's.
fn site_db(name: &str) -> Option<String> {
    let instance = name.strip_suffix(".localhost").filter(|id| valid_id(id));
    match instance.and_then(Site::instance) {
        Some(site) => Some(site.db),
        None => view_db(name),
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

    #[test]
    fn scratch_site_folders_name_their_database() {
        let v = Site::view(7);
        assert_eq!(view_db(&v.name), Some(v.db));
        assert_eq!(view_db("i1.localhost"), None);
        assert_eq!(view_db("view-x-1.localhost"), None);
    }

    #[test]
    fn site_folders_are_idle_only_when_their_database_is() {
        let v = Site::view(2);
        assert_eq!(site_db("i1.localhost").as_deref(), Some("eb_i1"));
        assert_eq!(site_db(&v.name), Some(v.db));
        assert_eq!(
            site_db("arena-base.localhost"),
            None,
            "the base site is never swept"
        );
        assert_eq!(site_db("Bad.localhost"), None);
        assert_eq!(site_db("apps.txt"), None);
    }
}
