//! Saves as manifests in `$ARENA_SAVES/mariadb/<save>/manifest.json`, dumps in `.blobs/`.
use std::path::Path;

use arena_part::helpers::mariadb::{saves_in, used_roots, Manifest, Tracked, TrackedChange};
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
        let dir = Path::new(&p.dir);
        let update = |c: TrackedChange| self.update_tracked(&p.instance, |t| c.apply(t));
        let bytes = self
            .dumps
            .save(
                &inst.server,
                &p.instance,
                &inst.tracked,
                inst.dump_after,
                dir,
                update,
            )
            .await?;
        self.dirs.note(&p.save, dir);
        Ok(Saved { bytes, exact: true })
    }

    /// Applies `f` to the live instance's tracking (nothing when it has stopped since).
    fn update_tracked(&self, id: &str, f: impl FnOnce(&mut Tracked)) {
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            f(&mut live.tracked);
        }
    }

    /// Removes dumps no save and no live instance needs; returns the bytes freed.
    pub(crate) async fn remove_unused_blobs(&self) -> Result<u64, PartError> {
        self.dumps
            .remove_unused(|| {
                let saves = self.manifests()?;
                let live = self.live.lock().expect("instances lock");
                Ok(used_roots(&saves, live.values().map(|i| &i.tracked.base)))
            })
            .await
    }
}
