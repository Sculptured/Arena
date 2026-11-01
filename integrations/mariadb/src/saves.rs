//! Saves as manifests in `$ARENA_SAVES/mariadb/<save>/manifest.json`, dumps in `.blobs/`.
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;

use arena_part::helpers::mariadb::{Manifest, Position, Segment};
use arena_part::proto::{valid_id, SaveParams, Saved};
use arena_part::PartError;

use crate::instances::Instance;
use crate::Mariadb;

impl Mariadb {
    /// The manifest of `save`; -32002 when the save has no folder.
    pub(crate) fn manifest(&self, save: &str) -> Result<Manifest, PartError> {
        let dir = self
            .dirs
            .find(save)
            .ok_or_else(|| PartError::unknown(format!("unknown save {save}")))?;
        Manifest::read(&dir).map_err(|e| PartError::internal(format!("save {save}: {e}")))
    }

    /// Every save on disk, by id. A folder without a manifest is not a save (dot folders such
    /// as `.blobs` neither); a manifest that can't be read is an error, so nothing is purged or
    /// swept on a guess.
    pub(crate) fn manifests(&self) -> Result<Vec<(String, Manifest)>, PartError> {
        let Ok(rd) = std::fs::read_dir(self.dirs.root()) else {
            return Ok(vec![]);
        };
        let mut out = vec![];
        for name in rd
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
        {
            if !valid_id(&name) {
                continue;
            }
            match Manifest::read(&self.dirs.root().join(&name)) {
                Ok(m) => out.push((name, m)),
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(PartError::internal(format!("save {name}: {e}"))),
            }
        }
        Ok(out)
    }

    /// The instance's base plus its changes since `start`; a dump instead when restoring that
    /// would read more than the instance's `dump_after_bytes` of binlog.
    pub(crate) async fn save_instance(&self, p: SaveParams) -> Result<Saved, PartError> {
        let inst = self.instance(&p.instance)?;
        let now = inst.server.position().await?;
        self.check_segment(&p.instance, &inst, &now).await?;
        let files = inst.server.binlog_files().await?;
        let own = Segment {
            db: inst.db.clone(),
            from: inst.start.clone(),
            to: now,
        };
        let m = inst.base.then(own);
        if !m.needs_dump(&files, inst.dump_after) {
            return self.write_save(&p, &m, 0);
        }
        // Held until the manifest naming the new dump is written, so no sweep takes it.
        let _gate = self.blob_gate.lock().await;
        let m = self.rebase(&p.instance, &inst).await?;
        let blob_bytes = m.root.as_deref().map_or(0, |id| self.blobs.size(id));
        self.write_save(&p, &m, blob_bytes)
    }

    /// -32001 when, since the last check, DDL changed the instance's database while another
    /// default database (or none) was selected, as a `connect` client can: its segment would miss
    /// that change. Otherwise the instance is checked up to `now`.
    async fn check_segment(
        &self,
        id: &str,
        inst: &Instance,
        now: &Position,
    ) -> Result<(), PartError> {
        let found = inst
            .server
            .misplaced_queries(&inst.db, &inst.checked, now)
            .await?;
        if !found.is_empty() {
            return Err(PartError::cant_restore(format!(
                "changes to {id} were made while another database was selected, so this save \
                 can't be exact ({})",
                found[0]
            )));
        }
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            live.checked.clone_from(now);
        }
        Ok(())
    }

    fn write_save(&self, p: &SaveParams, m: &Manifest, extra: u64) -> Result<Saved, PartError> {
        let dir = Path::new(&p.dir);
        let bytes = m.write(dir).map_err(PartError::internal_from)?;
        self.dirs.note(&p.save, dir);
        Ok(Saved {
            bytes: bytes + extra,
            exact: true,
        })
    }

    /// Dumps the instance as its new base; its own changes now start where the dump is
    /// consistent.
    async fn rebase(&self, id: &str, inst: &Instance) -> Result<Manifest, PartError> {
        let dumped = inst.server.dump(&inst.db, &self.blobs).await?;
        let m = Manifest::rooted(inst.server_ref.clone(), dumped.blob);
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            live.base = m.clone();
            live.checked.clone_from(&dumped.at);
            live.start = dumped.at;
        }
        Ok(m)
    }

    /// Removes dumps no save and no live instance needs; returns the bytes freed.
    pub(crate) async fn remove_unused_blobs(&self) -> Result<u64, PartError> {
        let _gate = self.blob_gate.lock().await;
        let saves = self.manifests()?;
        let mut used: HashSet<String> = saves.iter().filter_map(|(_, m)| m.root.clone()).collect();
        let live = self.live.lock().expect("instances lock");
        used.extend(live.values().filter_map(|i| i.base.root.clone()));
        drop(live);
        let freed = self.blobs.remove_unreferenced(&used);
        Ok(freed.map_err(PartError::internal_from)?.bytes)
    }
}
