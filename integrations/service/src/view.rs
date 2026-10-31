//! `inspect` and `diff` over volumes: a summary and a content hash per volume.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use arena_part::helpers::folder::{self, Digest};
use arena_part::proto::Docs;
use arena_part::PartError;
use serde_json::{json, Map, Value};

fn internal(e: impl std::fmt::Display) -> PartError {
    PartError::internal(e.to_string())
}

/// The key a volume has in `inspect` and `diff`.
pub fn volume_key(volume: &str) -> String {
    format!("volume/{volume}")
}

/// The digest of every volume folder under `root` (a save, or a live export), by key.
pub fn digests(root: &Path) -> Result<BTreeMap<String, Digest>, PartError> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(root).map_err(internal)? {
        let entry = entry.map_err(internal)?;
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            out.insert(
                volume_key(&name),
                folder::digest(&entry.path()).map_err(internal)?,
            );
        }
    }
    Ok(out)
}

/// A target's volumes by key.
pub type DigestMap = BTreeMap<String, Digest>;

fn doc(d: &Digest) -> Value {
    json!({"bytes": d.bytes, "files": d.files, "hash": d.hash})
}

/// `keys` (every volume when empty) of the digests; a key that is no volume is null.
pub fn inspect(digests: &BTreeMap<String, Digest>, keys: &[String]) -> Docs {
    let keys: Vec<&String> = if keys.is_empty() {
        digests.keys().collect()
    } else {
        keys.iter().collect()
    };
    let docs: Map<String, Value> = keys
        .into_iter()
        .map(|k| (k.clone(), digests.get(k).map_or(Value::Null, doc)))
        .collect();
    Docs { docs }
}

/// The volume keys whose content differs between two sets of digests.
pub fn diff(a: &BTreeMap<String, Digest>, b: &BTreeMap<String, Digest>) -> Vec<String> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter(|k| a.get(*k).map(|d| &d.hash) != b.get(*k).map(|d| &d.hash))
        .cloned()
        .collect()
}
