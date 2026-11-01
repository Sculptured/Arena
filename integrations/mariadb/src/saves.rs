//! Saves as manifests in `$ARENA_SAVES/mariadb/<save>/manifest.json`, dumps in `.blobs/`.
use std::collections::HashSet;
use std::path::Path;

use arena_part::helpers::mariadb::{Manifest, Segment};
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

    /// Every readable save on disk, by id (dot folders such as `.blobs` are not saves).
    pub(crate) fn manifests(&self) -> Vec<(String, Manifest)> {
        let Ok(rd) = std::fs::read_dir(self.dirs.root()) else {
            return vec![];
        };
        rd.filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| valid_id(name))
            .filter_map(|name| Some((name.clone(), self.manifest(&name).ok()?)))
            .collect()
    }

    /// The instance's base plus its changes since `start`; a dump instead when restoring that
    /// would read more than the instance's `dump_after_bytes` of binlog.
    pub(crate) async fn save_instance(&self, p: SaveParams) -> Result<Saved, PartError> {
        let inst = self.instance(&p.instance)?;
        let now = inst.server.position().await?;
        let files = inst.server.binlog_files().await?;
        let own = Segment {
            db: inst.db.clone(),
            from: inst.start.clone(),
            to: now.clone(),
        };
        let mut m = inst.base.then(own);
        let mut blob_bytes = 0;
        if m.needs_dump(&files, inst.dump_after) {
            m = self.rebase(&p.instance, &inst, now).await?;
            blob_bytes = m.root.as_deref().map_or(0, |id| self.blobs.size(id));
        }
        let dir = Path::new(&p.dir);
        let bytes = m.write(dir).map_err(PartError::internal_from)?;
        self.dirs.note(&p.save, dir);
        Ok(Saved {
            bytes: bytes + blob_bytes,
            exact: true,
        })
    }

    /// Dumps the instance as its new base; its own changes now start at `now`.
    async fn rebase(
        &self,
        id: &str,
        inst: &Instance,
        now: arena_part::helpers::mariadb::Position,
    ) -> Result<Manifest, PartError> {
        // Hold the gate until a live instance refers to the new dump, so no blob sweep runs in
        // between and takes it for unused.
        let _gate = self.blob_gate.lock().await;
        let blob = inst.server.dump(&inst.db, &self.blobs).await?;
        let m = Manifest::rooted(inst.server.reference(), blob);
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            live.base = m.clone();
            live.start = now;
        }
        Ok(m)
    }

    pub(crate) async fn drop_save_files(&self, save: &str) -> Result<(), PartError> {
        self.dirs.remove(save).map_err(PartError::internal_from)?;
        self.remove_unused_blobs().await.map(drop)
    }

    /// Removes dumps no save and no live instance needs; returns the bytes freed.
    pub(crate) async fn remove_unused_blobs(&self) -> Result<u64, PartError> {
        let _gate = self.blob_gate.lock().await;
        let mut used: HashSet<String> = self
            .manifests()
            .into_iter()
            .filter_map(|(_, m)| m.root)
            .collect();
        let live = self.live.lock().expect("instances lock");
        used.extend(live.values().filter_map(|i| i.base.root.clone()));
        drop(live);
        let freed = self.blobs.remove_unreferenced(&used);
        Ok(freed.map_err(PartError::internal_from)?.bytes)
    }
}
