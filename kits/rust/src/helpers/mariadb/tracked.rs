//! A live database whose saves are what it started from plus its own binlog segment, and the
//! steps of saving it.
use std::fmt;

use anyhow::Result;

use super::blobs::Blobs;
use super::dumps::Dumped;
use super::manifest::{Manifest, Segment};
use super::position::Position;
use super::server::Server;

/// A database being saved by binlog: its `base` (what it was built from), where its own changes
/// start, and how far they are known to be all in its own segment (see [`Server::chain`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub db: String,
    pub base: Manifest,
    pub start: Position,
    pub checked: Position,
}

/// DDL changed the database while another default database (or none) was selected, so its own
/// segment would miss that change. Holds the first such statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misplaced(pub String);

impl fmt::Display for Misplaced {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "changed while another database was selected ({})",
            self.0
        )
    }
}

impl std::error::Error for Misplaced {}

/// What saving a tracked database now gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chained {
    /// The base followed by the database's own segment up to now.
    pub manifest: Manifest,
    /// The database is checked up to here.
    pub checked: Position,
    /// Restoring `manifest` would read more binlog than allowed: dump instead ([`Server::rebase`]).
    pub needs_dump: bool,
}

impl Tracked {
    /// `db` built from `base`; its own changes start at `start`.
    pub fn new(db: &str, base: Manifest, start: Position) -> Tracked {
        Tracked {
            db: db.to_string(),
            base,
            checked: start.clone(),
            start,
        }
    }

    /// The save of the database as it is at binlog position `now`.
    pub fn manifest_at(&self, now: Position) -> Manifest {
        self.base.then(Segment {
            db: self.db.clone(),
            from: self.start.clone(),
            to: now,
        })
    }

    /// The binlog positions a later save still needs.
    pub fn needed(&self) -> impl Iterator<Item = &Position> {
        self.base.oldest().into_iter().chain([&self.start])
    }

    /// The database with the dump `d` as its new base: its own changes start where `d` is
    /// consistent.
    pub fn rebased(&self, d: Dumped) -> Tracked {
        let base = Manifest::rooted(self.base.server.clone(), d.blob);
        Tracked::new(&self.db, base, d.at)
    }
}

impl Server {
    /// The save of `t` now. Fails with [`Misplaced`] when, since `t.checked`, DDL changed the
    /// database under another default database.
    pub async fn chain(&self, t: &Tracked, dump_after: u64) -> Result<Chained> {
        let now = self.position().await?;
        let found = self.misplaced_queries(&t.db, &t.checked, &now).await?;
        if let Some(first) = found.into_iter().next() {
            return Err(Misplaced(first).into());
        }
        let files = self.binlog_files().await?;
        let manifest = t.manifest_at(now.clone());
        Ok(Chained {
            needs_dump: manifest.needs_dump(&files, dump_after),
            manifest,
            checked: now,
        })
    }

    /// Dumps `t`'s database into `blobs`; the result has that dump as its base. Hold whatever
    /// keeps unused blobs from being swept until a save names the dump.
    pub async fn rebase(&self, t: &Tracked, blobs: &Blobs) -> Result<Tracked> {
        Ok(t.rebased(self.dump(&t.db, blobs).await?))
    }
}
