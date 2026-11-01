//! Dumps into blobs, loading them, and rebuilding a database from a manifest.
use std::io::{self, Read};

use anyhow::{Context, Result};

use super::blobs::Blobs;
use super::docker;
use super::login::{replay_user, Login, REPLAY_GRANTS};
use super::manifest::Manifest;
use super::position::{dump_position, Position};
use super::server::{check_db, Server};

/// The dump notes its binlog position near its top; this much of it is kept to read it.
const HEAD: usize = 64 * 1024;

/// A dump stored as a blob, and the binlog position it is consistent with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dumped {
    pub blob: String,
    pub at: Position,
}

/// A reader that keeps a copy of the first `HEAD` bytes it passes on.
struct HeadReader<R> {
    inner: R,
    head: Vec<u8>,
}

impl<R: Read> Read for HeadReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let room = HEAD.saturating_sub(self.head.len()).min(n);
        self.head.extend_from_slice(&buf[..room]);
        Ok(n)
    }
}

/// Stores the dump read from `r` in `blobs`, with the position its head notes.
fn store(blobs: &Blobs, r: &mut dyn Read) -> io::Result<Dumped> {
    let mut head = HeadReader {
        inner: r,
        head: vec![],
    };
    let blob = blobs.write_from(&mut head)?;
    let at = dump_position(&String::from_utf8_lossy(&head.head))
        .ok_or_else(|| io::Error::other("the dump does not note its binlog position"))?;
    Ok(Dumped { blob, at })
}

impl Server {
    /// Dumps `db` into `blobs` (one consistent snapshot, with the binlog position it matches).
    pub async fn dump(&self, db: &str, blobs: &Blobs) -> Result<Dumped> {
        check_db(db)?;
        let (cfg, db, blobs) = (self.config().clone(), db.to_string(), blobs.clone());
        let task = move || docker::dump(&cfg, &db, |r| store(&blobs, r));
        tokio::task::spawn_blocking(task).await?
    }

    /// Runs the dump `id` from `blobs` in `db` as `login`, unlogged.
    pub async fn load(&self, blobs: &Blobs, id: &str, db: &str, login: &Login) -> Result<()> {
        check_db(db)?;
        let mut src = blobs.open(id).with_context(|| format!("dump {id}"))?;
        let (cfg, db, login) = (self.config().clone(), db.to_string(), login.clone());
        let task = move || docker::load(&cfg, &login, &db, &mut src);
        tokio::task::spawn_blocking(task).await?
    }

    /// Builds `target` from `m`: a new database, the root dump, then each segment, all run by an
    /// account limited to `target`. Returns the binlog position after, where `target`'s own
    /// changes begin. `target` is dropped on failure; the replay account always.
    pub async fn restore(&self, m: &Manifest, blobs: &Blobs, target: &str) -> Result<Position> {
        self.create_db(target).await?;
        let user = replay_user(target);
        let built = match self.create_login(&user, target, REPLAY_GRANTS).await {
            Ok(login) => self.rebuild(m, blobs, target, &login).await,
            Err(e) => Err(e),
        };
        let dropped = self.drop_login(&user).await;
        if built.is_err() {
            let _ = self.drop_db(target).await;
        }
        built?;
        dropped?;
        self.position().await
    }

    async fn rebuild(
        &self,
        m: &Manifest,
        blobs: &Blobs,
        target: &str,
        login: &Login,
    ) -> Result<()> {
        if let Some(root) = &m.root {
            self.load(blobs, root, target, login).await?;
        }
        for seg in &m.segments {
            self.apply_log(seg, target, login).await?;
        }
        Ok(())
    }
}
