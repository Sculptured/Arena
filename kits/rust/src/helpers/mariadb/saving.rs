//! Saving tracked databases: the steps every integration on this helper takes (chain, else dump
//! under the dump gate), and what the saves on disk and the live databases still need.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tokio::sync::{Mutex, MutexGuard};

use super::blobs::Blobs;
use super::checks::chain_error;
use super::manifest::{Manifest, ServerRef};
use super::position::{span_bytes, BinlogFile, Position};
use super::server::Server;
use super::tracked::Tracked;
use crate::proto::kept;
use crate::PartError;

/// The binlog bytes that saves kept by a pattern may hold on their own before they are re-rooted.
pub const DEFAULT_REROOT_AFTER: u64 = 256 * 1024 * 1024;

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

/// What re-rooting looks at: the saves on disk (in `root`), the `gc` `keep` list, the positions
/// live databases need, how many bytes to allow, and the scratch database to rebuild saves in.
pub struct Standing<'a> {
    pub root: &'a Path,
    pub saves: &'a [(String, Manifest)],
    pub keep: &'a [String],
    pub live: Vec<Position>,
    pub limit: u64,
    pub scratch: &'a str,
}

/// The saves on the server `here` to rewrite as dumps. A save kept by a pattern in `keep` (`base_*`,
/// `gold_*`) stands for as long as the benchmark does, so it would hold the binlog forever; every
/// other save and live database goes away in time. So: when the standing saves alone hold more than
/// `limit` bytes of binlog before anything else needs it, all that hold some of those bytes.
pub fn to_reroot<'a>(
    here: &ServerRef,
    standing: &Standing<'a>,
    files: &[BinlogFile],
    end: &Position,
) -> Vec<&'a (String, Manifest)> {
    let patterns: Vec<&String> = standing.keep.iter().filter(|k| k.ends_with('*')).collect();
    let (held, others): (Vec<_>, Vec<_>) = standing
        .saves
        .iter()
        .filter(|(_, m)| &m.server == here)
        .partition(|(id, _)| kept(&patterns, id));
    let others = others.iter().filter_map(|(_, m)| m.oldest());
    let floor = others.chain(&standing.live).min().unwrap_or(end);
    let holds = |m: &Manifest| m.oldest().is_some_and(|p| p < floor);
    let held: Vec<_> = held.into_iter().filter(|(_, m)| holds(m)).collect();
    let oldest = held.iter().filter_map(|(_, m)| m.oldest()).min();
    match oldest {
        Some(p) if span_bytes(files, p, floor) > standing.limit => held,
        _ => vec![],
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

    /// Rewrites the saves [`to_reroot`] picks as full dumps, so the binlog they held can go. A live
    /// database started from one keeps its own copy of the old manifest. Returns how many.
    pub async fn reroot_standing(
        &self,
        server: &Server,
        standing: &Standing<'_>,
    ) -> Result<usize, PartError> {
        let here = server.reference().await?;
        let files = server.binlog_files().await?;
        let end = server.position().await?;
        let picked = to_reroot(&here, standing, &files, &end);
        for (id, m) in &picked {
            let dir = standing.root.join(id);
            self.reroot(server, &here, m, &dir, standing.scratch)
                .await?;
        }
        Ok(picked.len())
    }

    /// `m`, rebuilt in the database `scratch` and dumped, becomes the save in `dir`. Under the gate,
    /// so no sweep takes the old root or the new dump meanwhile.
    async fn reroot(
        &self,
        server: &Server,
        here: &ServerRef,
        m: &Manifest,
        dir: &Path,
        scratch: &str,
    ) -> Result<(), PartError> {
        let _gate = self.gate.lock().await;
        server.restore(m, &self.blobs, scratch).await?;
        let dumped = server.dump(scratch, &self.blobs).await;
        let dropped = server.drop_db(scratch).await;
        let blob = dumped?.blob;
        dropped?;
        let rooted = Manifest::rooted(here.clone(), blob);
        rooted.write(dir).map_err(PartError::internal_from)?;
        Ok(())
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
