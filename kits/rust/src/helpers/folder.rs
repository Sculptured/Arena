//! Copying a part's folder into saves and back, and comparing folders.
//!
//! A save hard-links each file whose size and modification time match the previous save's copy;
//! a restore always copies, so the live folder can never change a save. Symlinks are copied as
//! symlinks and never followed.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// What a folder holds: file count, bytes and one hash over every path and file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub bytes: u64,
    pub files: u64,
    pub hash: String,
}

/// Copies `src` into `dst`. A file whose size and mtime match `prev`'s copy is hard-linked to it
/// instead of copied. Returns the bytes newly written.
pub fn copy_save(src: &Path, dst: &Path, prev: Option<&Path>) -> io::Result<u64> {
    copy_tree(src, dst, prev)
}

/// Copies the save `save` into the live folder `dst`. Never links.
pub fn restore(save: &Path, dst: &Path) -> io::Result<()> {
    copy_tree(save, dst, None).map(|_| ())
}

/// Bytes of the files under `path`, recursively (0 when it doesn't exist).
pub fn size(path: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(path) else {
        return 0;
    };
    rd.filter_map(Result::ok)
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// A regular file under a folder: its path relative to it (with `/`) and its absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub abs: PathBuf,
}

/// Every regular file under `root`, sorted by relative path.
pub fn list(root: &Path) -> io::Result<Vec<Entry>> {
    let mut out = vec![];
    collect(root, "", &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The content hash of every regular file under `root`, by path relative to it (with `/`).
pub fn file_hashes(root: &Path) -> io::Result<BTreeMap<String, String>> {
    list(root)?
        .into_iter()
        .map(|e| Ok((e.path, hash_file(&e.abs)?)))
        .collect()
}

/// Count, bytes and a content hash of the regular files under `root`.
pub fn digest(root: &Path) -> io::Result<Digest> {
    let hashes = file_hashes(root)?;
    let mut all = Sha256::new();
    for (path, hash) in &hashes {
        all.update(format!("{path}\0{hash}\n"));
    }
    Ok(Digest {
        bytes: size(root),
        files: hashes.len() as u64,
        hash: hex(&all.finalize()),
    })
}

fn copy_tree(src: &Path, dst: &Path, prev: Option<&Path>) -> io::Result<u64> {
    fs::create_dir_all(dst)?;
    let mut written = 0;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let (s, d) = (entry.path(), dst.join(&name));
        let p = prev.map(|p| p.join(&name));
        let kind = entry.file_type()?;
        written += if kind.is_dir() {
            copy_tree(&s, &d, p.as_deref())?
        } else if kind.is_symlink() {
            copy_link(&s, &d)?
        } else {
            copy_file(&s, &d, p.as_deref())?
        };
    }
    Ok(written)
}

/// Removes whatever is at `path` (a file or a link, not a folder) so it can be replaced.
fn clear(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn copy_link(src: &Path, dst: &Path) -> io::Result<u64> {
    clear(dst)?;
    symlink(fs::read_link(src)?, dst)?;
    Ok(0)
}

/// True when `prev` is a regular file with `meta`'s size and mtime.
fn unchanged(prev: &Path, meta: &fs::Metadata) -> bool {
    match fs::symlink_metadata(prev) {
        Ok(p) => p.is_file() && p.len() == meta.len() && p.modified().ok() == meta.modified().ok(),
        Err(_) => false,
    }
}

fn copy_file(src: &Path, dst: &Path, prev: Option<&Path>) -> io::Result<u64> {
    let mut input = File::open(src)?;
    let meta = input.metadata()?;
    clear(dst)?;
    if let Some(p) = prev.filter(|p| unchanged(p, &meta)) {
        fs::hard_link(p, dst)?;
        return Ok(0);
    }
    let mut out = File::create(dst)?;
    let n = io::copy(&mut input, &mut out)?;
    out.set_modified(meta.modified()?)?; // so the next save can link it
    out.set_permissions(meta.permissions())?;
    Ok(n)
}

fn collect(dir: &Path, rel: &str, out: &mut Vec<Entry>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = format!("{rel}{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), &format!("{path}/"), out)?;
        } else if kind.is_file() {
            out.push(Entry {
                path,
                abs: entry.path(),
            });
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut input = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match input.read(&mut buf)? {
            0 => return Ok(hex(&h.finalize())),
            n => h.update(&buf[..n]),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
