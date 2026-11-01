//! Dumps kept once each, named by the sha256 of their SQL: `<dir>/<id>.sql.gz`.
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

const SUFFIX: &str = ".sql.gz";

/// A folder of gzipped SQL dumps, each named by the hash of its content.
#[derive(Debug, Clone)]
pub struct Blobs {
    dir: PathBuf,
}

/// What `remove_unreferenced` removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Freed {
    pub count: u64,
    pub bytes: u64,
}

/// A blob id is a lowercase hex sha256, so it can never name a path outside the folder.
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn bad_id(id: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("bad blob id {id:?}"))
}

impl Blobs {
    pub fn new(dir: PathBuf) -> Blobs {
        Blobs { dir }
    }

    pub fn path(&self, id: &str) -> io::Result<PathBuf> {
        if !valid_id(id) {
            return Err(bad_id(id));
        }
        Ok(self.dir.join(format!("{id}{SUFFIX}")))
    }

    /// Stores the SQL read from `r`; returns its id. The same SQL is stored once.
    pub fn write_from(&self, mut r: impl Read) -> io::Result<String> {
        fs::create_dir_all(&self.dir)?;
        let tmp = self.temp_path();
        let id = match write_gz(&mut r, &tmp) {
            Ok(id) => id,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        };
        fs::rename(&tmp, self.path(&id)?)?;
        Ok(id)
    }

    /// The SQL of blob `id`.
    pub fn open(&self, id: &str) -> io::Result<impl Read> {
        let f = File::open(self.path(id)?)?;
        Ok(GzDecoder::new(BufReader::new(f)))
    }

    /// The stored (compressed) size of blob `id`, 0 when it is missing.
    pub fn size(&self, id: &str) -> u64 {
        self.path(id).and_then(fs::metadata).map_or(0, |m| m.len())
    }

    /// Removes every blob whose id is not in `referenced`.
    pub fn remove_unreferenced(&self, referenced: &HashSet<String>) -> io::Result<Freed> {
        let mut freed = Freed::default();
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Ok(freed);
        };
        for entry in rd.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(SUFFIX).filter(|id| valid_id(id)) else {
                continue;
            };
            if !referenced.contains(id) {
                freed.bytes += entry.metadata().map_or(0, |m| m.len());
                fs::remove_file(entry.path())?;
                freed.count += 1;
            }
        }
        Ok(freed)
    }

    /// A unique file to write a blob into before it has a name (not a blob name, so never swept
    /// as one).
    fn temp_path(&self) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!(".part_{}_{n}", std::process::id()))
    }
}

/// Gzips `r` into `path`; returns the sha256 of the uncompressed bytes.
fn write_gz(r: &mut impl Read, path: &Path) -> io::Result<String> {
    let mut gz = GzEncoder::new(File::create(path)?, Compression::fast());
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        gz.write_all(&buf[..n])?;
    }
    gz.finish()?.sync_all()?;
    Ok(hex(&hash.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
