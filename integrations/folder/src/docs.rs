//! `inspect` and `diff` over a folder.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use arena_part::helpers::folder;
use arena_part::proto::Docs;
use arena_part::PartError;
use serde_json::{json, Map, Value};

use crate::paths::resolve;
use crate::tools::file_key;

/// The document of a file key: its size and text (null content for a binary file).
fn doc(root: &Path, key: &str) -> Value {
    let Some(rel) = key.strip_prefix("file/") else {
        return Value::Null;
    };
    let Ok(at) = resolve(root, rel, false) else {
        return Value::Null;
    };
    match fs::read(&at.abs) {
        Ok(bytes) => json!({"bytes": bytes.len(), "content": String::from_utf8(bytes).ok()}),
        Err(_) => Value::Null,
    }
}

/// `keys` (every file when empty) of the folder `root`.
pub fn inspect(root: &Path, keys: &[String]) -> Result<Docs, PartError> {
    let keys: Vec<String> = if keys.is_empty() {
        let files = folder::list(root).map_err(PartError::internal_from)?;
        files.iter().map(|e| file_key(&e.path)).collect()
    } else {
        keys.to_vec()
    };
    let docs: Map<String, Value> = keys
        .into_iter()
        .map(|k| {
            let d = doc(root, &k);
            (k, d)
        })
        .collect();
    Ok(Docs { docs })
}

/// The file keys whose content differs between two folders, or that only one has.
pub fn diff(a: &Path, b: &Path) -> Result<Vec<String>, PartError> {
    let (ha, hb) = (
        folder::file_hashes(a).map_err(PartError::internal_from)?,
        folder::file_hashes(b).map_err(PartError::internal_from)?,
    );
    let paths: BTreeSet<&String> = ha.keys().chain(hb.keys()).collect();
    Ok(paths
        .into_iter()
        .filter(|p| ha.get(*p) != hb.get(*p))
        .map(|p| file_key(p))
        .collect())
}
