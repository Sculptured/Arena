//! Saves as manifests in `$ARENA_SAVES/mariadb/<save>/manifest.json`, dumps in `.blobs/`.
use std::collections::HashSet;
use std::path::Path;

use arena_part::helpers::mariadb::{chain_error, saves_in, Manifest, Tracked};
use arena_part::proto::{SaveParams, Saved};
use arena_part::PartError;

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

    /// Every save on disk, by id (see `saves_in`).
    pub(crate) fn manifests(&self) -> Result<Vec<(String, Manifest)>, PartError> {
        saves_in(self.dirs.root()).map_err(PartError::internal_from)
    }

    /// The instance's base plus its changes since its start; a dump instead when restoring that
    /// would read more than the instance's `dump_after_bytes` of binlog. -32001 when, since the
    /// last save, DDL changed the instance's database while another default database (or none)
    /// was selected, as a `connect` client can: its segment would miss that change.
    pub(crate) async fn save_instance(&self, p: SaveParams) -> Result<Saved, PartError> {
        let inst = self.instance(&p.instance)?;
        let chained = inst.server.chain(&inst.tracked, inst.dump_after).await;
        let chained = chained.map_err(|e| chain_error(&p.instance, e))?;
        self.update_tracked(&p.instance, |t| t.checked.clone_from(&chained.checked));
        if !chained.needs_dump {
            return self.write_save(&p, &chained.manifest, 0);
        }
        // Held until the manifest naming the new dump is written, so no sweep takes it.
        let _gate = self.blob_gate.lock().await;
        let rebased = inst.server.rebase(&inst.tracked, &self.blobs).await?;
        let m = rebased.base.clone();
        self.update_tracked(&p.instance, |t| *t = rebased);
        let blob_bytes = m.root.as_deref().map_or(0, |id| self.blobs.size(id));
        self.write_save(&p, &m, blob_bytes)
    }

    /// Applies `f` to the live instance's tracking (nothing when it has stopped since).
    fn update_tracked(&self, id: &str, f: impl FnOnce(&mut Tracked)) {
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            f(&mut live.tracked);
        }
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

    /// Removes dumps no save and no live instance needs; returns the bytes freed.
    pub(crate) async fn remove_unused_blobs(&self) -> Result<u64, PartError> {
        let _gate = self.blob_gate.lock().await;
        let saves = self.manifests()?;
        let mut used: HashSet<String> = saves.iter().filter_map(|(_, m)| m.root.clone()).collect();
        let live = self.live.lock().expect("instances lock");
        used.extend(live.values().filter_map(|i| i.tracked.base.root.clone()));
        drop(live);
        let freed = self.blobs.remove_unreferenced(&used);
        Ok(freed.map_err(PartError::internal_from)?.bytes)
    }
}
