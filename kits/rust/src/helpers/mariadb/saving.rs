//! Saving tracked databases: the steps every integration on this helper takes (chain, else dump
//! under the dump gate), and what the saves on disk and the live databases still need.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tokio::sync::{Mutex, MutexGuard};

use super::blobs::Blobs;
use super::checks::chain_error;
use super::manifest::{Manifest, ServerRef};
use super::position::Position;
use super::server::Server;
use super::tracked::Tracked;
use crate::PartError;

/// What a save changed about a live database's tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackedChange {
    /// Its changes are checked up to here.
    Checked(Position),
    /// It was dumped: this is its tracking now.
    Rebased(Tracked),
}

impl TrackedChange {
    pub fn apply(self, t: &mut Tracked) {
        match self {
            TrackedChange::Checked(p) => t.checked = p,
            TrackedChange::Rebased(r) => *t = r,
        }
    }
}

/// The dumps of one integration, and the gate that keeps a new dump from being swept as unused
/// before a manifest names it.
#[derive(Debug)]
pub struct Dumps {
    blobs: Blobs,
    gate: Mutex<()>,
}

impl Dumps {
    /// Dumps kept in `dir`.
    pub fn new(dir: PathBuf) -> Dumps {
        Dumps {
            blobs: Blobs::new(dir),
            gate: Mutex::new(()),
        }
    }

    pub fn blobs(&self) -> &Blobs {
        &self.blobs
    }

    /// The gate, for a dump made outside [`Dumps::save`]: hold it until a manifest names it.
    pub async fn lock(&self) -> MutexGuard<'_, ()> {
        self.gate.lock().await
    }

    /// Writes the save of `t` (instance `id` on `server`) into `dir`: its base and its changes
    /// since, or a dump when restoring that would read more than `dump_after` binlog bytes.
    /// `update` gets each change to apply to the live tracking. Returns the bytes written.
    pub async fn save(
        &self,
        server: &Server,
        id: &str,
        t: &Tracked,
        dump_after: u64,
        dir: &Path,
        mut update: impl FnMut(TrackedChange),
    ) -> Result<u64, PartError> {
        let chained = server.chain(t, dump_after).await;
        let chained = chained.map_err(|e| chain_error(id, e))?;
        update(TrackedChange::Checked(chained.checked.clone()));
        if !chained.needs_dump {
            return chained
                .manifest
                .write(dir)
                .map_err(PartError::internal_from);
        }
        // Held until the manifest naming the new dump is written, so no sweep takes it.
        let _gate = self.gate.lock().await;
        let rebased = server.rebase(t, &self.blobs).await?;
        let m = rebased.base.clone();
        update(TrackedChange::Rebased(rebased));
        let blob = m.root.as_deref().map_or(0, |b| self.blobs.size(b));
        Ok(blob + m.write(dir).map_err(PartError::internal_from)?)
    }

    /// Removes the dumps not in `used()` (read under the gate); returns the bytes freed.
    pub async fn remove_unused(
        &self,
        used: impl FnOnce() -> Result<HashSet<String>, PartError>,
    ) -> Result<u64, PartError> {
        let _gate = self.gate.lock().await;
        let freed = self.blobs.remove_unreferenced(&used()?);
        Ok(freed.map_err(PartError::internal_from)?.bytes)
    }
}

/// The dumps the saves and the `others` (live bases, a kept base) are rooted on.
pub fn used_roots<'a>(
    saves: &'a [(String, Manifest)],
    others: impl IntoIterator<Item = &'a Manifest>,
) -> HashSet<String> {
    let all = saves.iter().map(|(_, m)| m).chain(others);
    all.filter_map(|m| m.root.clone()).collect()
}

/// The binlog positions on the server `here` that its saves and the `live` databases (on it)
/// still need.
pub fn needed_positions<'a>(
    here: &ServerRef,
    saves: &'a [(String, Manifest)],
    live: impl IntoIterator<Item = &'a Tracked>,
) -> Vec<Position> {
    let mut out: Vec<Position> = saves
        .iter()
        .filter(|(_, m)| &m.server == here)
        .filter_map(|(_, m)| m.oldest().cloned())
        .collect();
    out.extend(live.into_iter().flat_map(|t| t.needed().cloned()));
    out
}
