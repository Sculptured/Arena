//! Binlog positions and files, and the arithmetic over them (no server needed).
use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A place in the binlog: a file (`binlog.000012`) and a byte offset in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub file: String,
    pub pos: u64,
}

impl Position {
    pub fn new(file: impl Into<String>, pos: u64) -> Position {
        Position {
            file: file.into(),
            pos,
        }
    }
}

/// Positions order by the file's sequence number, then by offset.
impl Ord for Position {
    fn cmp(&self, other: &Self) -> Ordering {
        (file_seq(&self.file), self.pos).cmp(&(file_seq(&other.file), other.pos))
    }
}

impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One binlog file as `SHOW BINARY LOGS` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinlogFile {
    pub name: String,
    pub size: u64,
}

impl BinlogFile {
    pub fn new(name: impl Into<String>, size: u64) -> BinlogFile {
        BinlogFile {
            name: name.into(),
            size,
        }
    }
}

/// The sequence number of a binlog file name: `binlog.000012` -> 12 (0 when there is none).
pub fn file_seq(name: &str) -> u64 {
    name.rsplit('.')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// A binlog file a segment needs is no longer on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purged(pub String);

impl fmt::Display for Purged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "binlog file {} is no longer on the server", self.0)
    }
}

impl std::error::Error for Purged {}

/// The files holding the events from `from` to `to`, in order.
pub fn span<'a>(
    files: &'a [BinlogFile],
    from: &Position,
    to: &Position,
) -> Result<Vec<&'a BinlogFile>, Purged> {
    for end in [from, to] {
        if !files.iter().any(|f| f.name == end.file) {
            return Err(Purged(end.file.clone()));
        }
    }
    let (lo, hi) = (file_seq(&from.file), file_seq(&to.file));
    let mut out: Vec<&BinlogFile> = files
        .iter()
        .filter(|f| (lo..=hi).contains(&file_seq(&f.name)))
        .collect();
    out.sort_by_key(|f| file_seq(&f.name));
    Ok(out)
}

/// The binlog bytes from `from` to `to` (all databases' events, so an upper bound for one).
pub fn span_bytes(files: &[BinlogFile], from: &Position, to: &Position) -> u64 {
    if from.file == to.file {
        return to.pos.saturating_sub(from.pos);
    }
    let (lo, hi) = (file_seq(&from.file), file_seq(&to.file));
    let size = |name: &str| files.iter().find(|f| f.name == name).map_or(0, |f| f.size);
    let middle: u64 = files
        .iter()
        .filter(|f| file_seq(&f.name) > lo && file_seq(&f.name) < hi)
        .map(|f| f.size)
        .sum();
    size(&from.file).saturating_sub(from.pos) + middle + to.pos
}

/// The file to `PURGE BINARY LOGS TO`, if any: the newest file on the server (`files`) not after
/// the one holding the oldest position still needed, or the current file when nothing is needed.
/// Only files before it are removed, so `None` when that is the first file on the server.
pub fn purge_target<'a>(
    needed: impl IntoIterator<Item = &'a Position>,
    current: &Position,
    files: &'a [BinlogFile],
) -> Option<&'a str> {
    let keep_from = file_seq(&needed.into_iter().min().unwrap_or(current).file);
    let target = files
        .iter()
        .filter(|f| file_seq(&f.name) <= keep_from)
        .max_by_key(|f| file_seq(&f.name))?;
    let first = files.iter().map(|f| file_seq(&f.name)).min();
    (Some(file_seq(&target.name)) != first).then_some(target.name.as_str())
}

/// The binlog position a `mariadb-dump --master-data=2` notes in its head
/// (`-- CHANGE MASTER TO MASTER_LOG_FILE='binlog.000002', MASTER_LOG_POS=841;`).
pub fn dump_position(head: &str) -> Option<Position> {
    let rest = head.split("MASTER_LOG_FILE='").nth(1)?;
    let (file, rest) = rest.split_once('\'')?;
    let rest = rest.split("MASTER_LOG_POS=").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some(Position::new(file, digits.parse().ok()?))
}
