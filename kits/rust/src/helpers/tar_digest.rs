//! Reading a tar archive without unpacking it: a summary and a content hash.
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use sha2::{Digest as _, Sha256};

use super::folder::{hex, Digest};

const BLOCK: usize = 512;

/// A tar header: the fields that name an entry and size its data.
struct Header([u8; BLOCK]);

impl Header {
    fn is_end(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    fn name(&self) -> String {
        let text = |r: std::ops::Range<usize>| {
            let f = &self.0[r];
            String::from_utf8_lossy(&f[..f.iter().position(|b| *b == 0).unwrap_or(f.len())])
                .into_owned()
        };
        format!("{}/{}", text(345..500), text(0..100))
    }

    fn is_dir(&self) -> bool {
        self.0[156] == b'5'
    }

    /// The header as hashed: a folder's mtime is left out, as unpacking does not keep it.
    fn hashed(&self) -> [u8; BLOCK] {
        let mut block = self.0;
        if self.is_dir() {
            block[136..156].fill(0); // mtime and the checksum that covers it
        }
        block
    }

    fn is_file(&self) -> bool {
        matches!(self.0[156], b'0' | 0)
    }

    /// The data size: octal text, or big-endian binary when the top bit is set.
    fn size(&self) -> u64 {
        let f = &self.0[124..136];
        if f[0] & 0x80 != 0 {
            return f[1..].iter().fold(0, |n, b| n << 8 | u64::from(*b));
        }
        let text: String = f
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| *b as char)
            .collect();
        u64::from_str_radix(text.trim(), 8).unwrap_or(0)
    }
}

/// Hashes the header block and `size` bytes of data (plus padding, which is skipped).
fn hash_entry<R: Read>(r: &mut R, header: &Header) -> io::Result<String> {
    let mut h = Sha256::new();
    h.update(header.hashed());
    let size = header.size();
    let mut left = size;
    let mut buf = [0u8; 64 * 1024];
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = r.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        h.update(&buf[..n]);
        left -= n as u64;
    }
    let pad = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
    io::copy(&mut r.take(pad), &mut io::sink())?;
    Ok(hex(&h.finalize()))
}

fn next_header<R: Read>(r: &mut R) -> io::Result<Option<Header>> {
    let mut block = [0u8; BLOCK];
    match r.read_exact(&mut block) {
        Ok(()) => Ok(Some(Header(block)).filter(|h| !h.is_end())),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}

/// Count and bytes of the regular files in the archive, and one hash over every entry (name, owner,
/// mode, mtime of files, data), whatever order they were written in.
pub fn digest(path: &Path) -> io::Result<Digest> {
    let mut r = BufReader::new(File::open(path)?);
    let (mut files, mut bytes) = (0, 0);
    let mut entries = vec![];
    while let Some(header) = next_header(&mut r)? {
        if header.is_file() {
            files += 1;
            bytes += header.size();
        }
        entries.push((header.name(), hash_entry(&mut r, &header)?));
    }
    entries.sort();
    let mut all = Sha256::new();
    for (name, hash) in &entries {
        all.update(format!("{name}\0{hash}\n"));
    }
    Ok(Digest {
        bytes,
        files,
        hash: hex(&all.finalize()),
    })
}
