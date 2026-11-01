//! Saves on disk, `$ARENA_SAVES/sculptured/<save>/`: `data/`, the server's data folder,
//! `changes.json`, the team's changes as the tools showed them (what `inspect` and `diff` read),
//! and `meta.json`, the sculptured build that wrote the store and the re-raised changes.
//!
//! Each SQLite store in the data folder (`controld.sqlite`, the runner's `hostd.sqlite`) is
//! copied by SQLite's online backup, a consistent copy of a store in use; the rest of the folder
//! by the kit's folder copy, without the stores' journals and what only a running server has
//! (its runner's pid, lock and address, its sockets). A restore copies `data/` back.
use std::path::Path;

use anyhow::{Context, Result};
use arena_part::helpers::folder;
use rusqlite::{Connection, OpenFlags, MAIN_DB};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aliases::Aliases;

const DATA: &str = "data";
const CHANGES: &str = "changes.json";
const META: &str = "meta.json";

/// What a save records besides the data folder and the changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// `sculptured --version` of the server that wrote the store.
    pub build: String,
    pub aliases: Aliases,
}
/// Files only a running server has, relative to its data folder.
const RUNTIME: [&str; 3] = [
    "host-local/hostd.pid",
    "host-local/hostd.lock",
    "host-local/controld.url",
];
const STORE: &str = ".sqlite";

/// True for what the folder copy leaves out: stores (backed up instead), their journals and the
/// runtime files.
fn left_out(rel: &str) -> bool {
    let journal = ["-wal", "-shm", "-journal"]
        .iter()
        .any(|j| rel.strip_suffix(j).is_some_and(|s| s.ends_with(STORE)));
    rel.ends_with(STORE) || journal || RUNTIME.contains(&rel)
}

/// Writes the save of the data folder `data` to `dir`, with `changes` and `meta`; returns its
/// bytes. A failed save leaves no folder.
pub fn write(data: &Path, changes: &[Value], meta: &Meta, dir: &Path) -> Result<u64> {
    let written = (|| {
        folder::copy_save_except(data, &dir.join(DATA), None, &left_out)?;
        backup_stores(data, &dir.join(DATA))?;
        std::fs::write(dir.join(CHANGES), serde_json::to_vec(changes)?)?;
        std::fs::write(dir.join(META), serde_json::to_vec(meta)?)?;
        Ok(folder::size(dir))
    })();
    if written.is_err() {
        folder::discard(dir);
    }
    written
}

/// Backs up every store under `data` to the same place under `to`.
fn backup_stores(data: &Path, to: &Path) -> Result<()> {
    for e in folder::list(data)? {
        if !e.path.ends_with(STORE) {
            continue;
        }
        let src = Connection::open_with_flags(&e.abs, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        src.busy_timeout(std::time::Duration::from_secs(10))?;
        let dst = to.join(&e.path);
        std::fs::create_dir_all(dst.parent().expect("a store path has a folder"))?;
        src.backup(MAIN_DB, &dst, None)
            .with_context(|| format!("backing up {}", e.path))?;
    }
    Ok(())
}

/// The changes recorded in the save `dir`.
pub fn changes(dir: &Path) -> Result<Vec<Value>> {
    let text = std::fs::read(dir.join(CHANGES)).context("reading the save's changes")?;
    Ok(serde_json::from_slice(&text)?)
}

/// What the save `dir` records besides its data folder and changes.
pub fn meta(dir: &Path) -> Result<Meta> {
    let text = std::fs::read(dir.join(META)).context("reading the save's meta.json")?;
    Ok(serde_json::from_slice(&text)?)
}

/// Copies the save `dir`'s data folder to `data`.
pub fn restore(dir: &Path, data: &Path) -> Result<()> {
    folder::restore(&dir.join(DATA), data).context("copying the save's data folder")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_part::helpers::temp_dir::TempDir;

    #[test]
    fn left_out_files() {
        for rel in [
            "controld.sqlite",
            "controld.sqlite-wal",
            "host-local/hostd.sqlite-shm",
            "host-local/hostd.pid",
            "host-local/controld.url",
        ] {
            assert!(left_out(rel), "{rel}");
        }
        for rel in ["host-token", "host-local", "notes-wal", "x.sqlite.txt"] {
            assert!(!left_out(rel), "{rel}");
        }
    }

    #[test]
    fn a_save_backs_up_stores_and_restores() {
        let tmp = TempDir::new_in(&std::env::temp_dir(), "arena_sc_saves").unwrap();
        let root = tmp.path();
        let (data, dir, back) = (root.join("data"), root.join("s1"), root.join("back"));
        std::fs::create_dir_all(data.join("host-local")).unwrap();
        std::fs::write(data.join("host-token"), "t").unwrap();
        std::fs::write(data.join("host-local/hostd.pid"), "1").unwrap();
        let db = Connection::open(data.join("controld.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE e(x); INSERT INTO e VALUES (7);")
            .unwrap();
        let changes = vec![serde_json::json!({"id": "chg-1"})];
        let mut aliases = Aliases::default();
        aliases.add("chg-0", "chg-1");
        let meta = Meta {
            build: "sculptured 0.1.0".into(),
            aliases,
        };
        assert!(write(&data, &changes, &meta, &dir).unwrap() > 0);
        assert!(!dir.join("data/controld.sqlite-wal").exists());
        drop(db);
        restore(&dir, &back).unwrap();
        let copy = Connection::open(back.join("controld.sqlite")).unwrap();
        let x: i64 = copy.query_row("SELECT x FROM e", [], |r| r.get(0)).unwrap();
        assert_eq!(x, 7);
        assert!(back.join("host-token").is_file() && !back.join("host-local/hostd.pid").exists());
        assert_eq!(super::changes(&dir).unwrap(), changes);
        assert_eq!(super::meta(&dir).unwrap(), meta);
    }
}
