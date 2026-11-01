//! A save as a chain: an optional dump (the root) and the binlog segments to apply on top.
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::position::{span_bytes, BinlogFile, Position};

/// The file a save's manifest is kept in, in the save's folder.
pub const MANIFEST: &str = "manifest.json";

/// The server whose binlog a manifest's positions refer to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerRef {
    pub container: String,
    pub port: u16,
}

/// The events of database `db` from `from` to `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub db: String,
    pub from: Position,
    pub to: Position,
}

/// How to rebuild a database: load the dump `root` (a blob id), if any, into an empty database,
/// then apply each segment in order, renamed to the new database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub server: ServerRef,
    pub root: Option<String>,
    pub segments: Vec<Segment>,
}

impl Manifest {
    /// An empty database.
    pub fn empty(server: ServerRef) -> Manifest {
        Manifest {
            server,
            root: None,
            segments: vec![],
        }
    }

    /// The database in the dump `root`.
    pub fn rooted(server: ServerRef, root: String) -> Manifest {
        Manifest {
            server,
            root: Some(root),
            segments: vec![],
        }
    }

    /// This manifest followed by `seg` (unchanged when `seg` holds no events).
    pub fn then(&self, seg: Segment) -> Manifest {
        let mut out = self.clone();
        if seg.from != seg.to {
            out.segments.push(seg);
        }
        out
    }

    /// The binlog bytes a restore reads (an upper bound: other databases' events are counted).
    pub fn binlog_bytes(&self, files: &[BinlogFile]) -> u64 {
        self.segments
            .iter()
            .map(|s| span_bytes(files, &s.from, &s.to))
            .sum()
    }

    /// Whether restoring would read more than `limit` binlog bytes, so a dump should replace it.
    pub fn needs_dump(&self, files: &[BinlogFile], limit: u64) -> bool {
        self.binlog_bytes(files) > limit
    }

    /// The oldest binlog position a restore needs.
    pub fn oldest(&self) -> Option<&Position> {
        self.segments.iter().map(|s| &s.from).min()
    }

    /// Reads `<dir>/manifest.json`.
    pub fn read(dir: &Path) -> io::Result<Manifest> {
        let text = std::fs::read(dir.join(MANIFEST))?;
        serde_json::from_slice(&text).map_err(io::Error::other)
    }

    /// Writes `<dir>/manifest.json` (creating `dir`); returns its size.
    pub fn write(&self, dir: &Path) -> io::Result<u64> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        std::fs::write(dir.join(MANIFEST), &text)?;
        Ok(text.len() as u64)
    }
}
