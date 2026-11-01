//! JSON files: config files, the base's files and a save's `site.json`.
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)
}

/// Writes `v` (pretty) to `path`; returns the bytes written.
pub fn write<T: Serialize>(path: &Path, v: &T) -> io::Result<u64> {
    let text = serde_json::to_vec_pretty(v).map_err(io::Error::other)?;
    std::fs::write(path, &text)?;
    Ok(text.len() as u64)
}
