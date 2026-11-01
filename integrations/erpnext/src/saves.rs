//! Saves in `$ARENA_SAVES/erpnext/<save>/`: `manifest.json` (the database, written last),
//! `files/` (the site's public and private files) and `site.json` (what `diff` compares).
use std::io;
use std::path::{Path, PathBuf};

use arena_part::helpers::folder;
use arena_part::helpers::mariadb::{saves_in, used_roots, Manifest, TrackedChange};
use arena_part::proto::{SaveParams, Saved};
use arena_part::PartError;
use serde::{Deserialize, Serialize};

use crate::instances::Instance;
use crate::jobs::{drain, JOBS_TIMEOUT};
use crate::json_file;
use crate::ready::Ready;
use crate::Erpnext;

const SITE_META: &str = "site.json";

/// What a save records about its instance's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveMeta {
    pub diff_doctypes: Vec<String>,
}

impl SaveMeta {
    pub fn read(dir: &Path) -> io::Result<SaveMeta> {
        json_file::read(&dir.join(SITE_META))
    }

    /// Writes it into `dir`; returns its size.
    fn write(&self, dir: &Path) -> io::Result<u64> {
        json_file::write(&dir.join(SITE_META), self)
    }
}

impl Erpnext {
    /// Every save on disk, by id.
    pub(crate) fn manifests(&self) -> Result<Vec<(String, Manifest)>, PartError> {
        saves_in(self.dirs.root()).map_err(PartError::internal_from)
    }

    /// The base's manifest, when there is a base.
    pub(crate) fn base_manifest(&self) -> Option<Manifest> {
        Manifest::read(&self.dirs.root().join(".base")).ok()
    }

    /// Waits for the site's background jobs, then saves its files and its database.
    pub(crate) async fn save_instance(&self, p: SaveParams) -> Result<Saved, PartError> {
        let ready = self.ready().await?;
        let inst = self.instance(&p.instance)?;
        drain(&ready.rest, &inst.site.name, JOBS_TIMEOUT).await?;
        let dir = PathBuf::from(&p.dir);
        let written = self.write_save(ready, &p.instance, &inst, &dir).await;
        if written.is_err() {
            folder::discard(&dir);
        }
        let bytes = written?;
        self.dirs.note(&p.save, &dir);
        self.update(&p.instance, |i| i.prev_files = dir.join("files"));
        Ok(Saved { bytes, exact: true })
    }

    /// Writes the save's files, then its manifest (see `Dumps::save`); returns the bytes
    /// written.
    async fn write_save(
        &self,
        ready: &Ready,
        id: &str,
        inst: &Instance,
        dir: &Path,
    ) -> Result<u64, PartError> {
        let files = write_files(ready, inst, dir).map_err(PartError::internal_from)?;
        let update = |c: TrackedChange| self.update(id, |i| c.apply(&mut i.tracked));
        let server = &ready.stack.server;
        let dump_after = inst.settings.dump_after_bytes;
        let manifest = self
            .dumps
            .save(server, id, &inst.tracked, dump_after, dir, update);
        Ok(files + manifest.await?)
    }

    /// Applies `f` to the live instance (nothing when it has stopped since).
    fn update(&self, id: &str, f: impl FnOnce(&mut Instance)) {
        if let Some(live) = self.live.lock().expect("instances lock").get_mut(id) {
            f(live);
        }
    }

    /// Removes dumps that no save, no live instance and not the base needs; the bytes freed.
    pub(crate) async fn remove_unused_blobs(&self) -> Result<u64, PartError> {
        self.dumps
            .remove_unused(|| {
                let saves = self.manifests()?;
                let base = self.base_manifest();
                let live = self.live.lock().expect("instances lock");
                let bases = live.values().map(|i| &i.tracked.base);
                Ok(used_roots(&saves, bases.chain(base.as_ref())))
            })
            .await
    }
}

/// The site's files and what `diff` compares; the bytes written.
fn write_files(ready: &Ready, inst: &Instance, dir: &Path) -> io::Result<u64> {
    let prev = Some(inst.prev_files.as_path());
    let files = ready
        .stack
        .sites
        .save_files(&inst.site.name, &dir.join("files"), prev)?;
    let meta = SaveMeta {
        diff_doctypes: inst.settings.diff_doctypes.clone(),
    };
    Ok(files + meta.write(dir)?)
}
