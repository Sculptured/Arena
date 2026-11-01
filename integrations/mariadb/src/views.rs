//! `inspect` and `diff`: an instance is read in place; a save is rebuilt into a scratch database
//! `mx_<pid>_<n>` (unlogged) for the read and dropped after.
use std::sync::atomic::{AtomicU64, Ordering};

use arena_part::helpers::mariadb::mysql_async::Conn;
use arena_part::helpers::mariadb::{check_same_server, restore_error, Server};
use arena_part::proto::{DiffParams, Differing, Docs, InspectParams, Target};
use arena_part::PartError;

use crate::{rows, Mariadb};

/// Scratch databases for reading saves start with this.
pub const VIEW_PREFIX: &str = "mx_";

/// A database to read from; `view` when it is a scratch copy of a save.
struct Opened {
    server: Server,
    db: String,
    view: bool,
}

fn view_name() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{VIEW_PREFIX}{}_{n}", std::process::id())
}

impl Mariadb {
    /// The database to read; a save is rebuilt (the caller marks `db` busy first).
    async fn open(&self, t: &Target, db: &str) -> Result<Opened, PartError> {
        let save = match t {
            Target::Instance { instance } => {
                let i = self.instance(instance)?;
                return Ok(Opened {
                    server: i.server,
                    db: i.tracked.db,
                    view: false,
                });
            }
            Target::Save { save } => save,
        };
        let m = self.manifest(save)?;
        let server = self.servers.for_ref(&m.server)?;
        check_same_server(save, &m.server, &server.reference().await?)?;
        let built = server.restore(&m, &self.blobs, db).await;
        built.map_err(|e| restore_error(save, e))?;
        Ok(Opened {
            server,
            db: db.to_string(),
            view: true,
        })
    }

    /// Runs `f` on a connection to the target's database.
    async fn read<T>(
        &self,
        t: &Target,
        f: impl AsyncFnOnce(&mut Conn) -> anyhow::Result<T>,
    ) -> Result<T, PartError> {
        let name = view_name();
        let _busy = self.busy.mark(&name);
        let o = self.open(t, &name).await?;
        let out = o.server.with_db(&o.db, f).await;
        if o.view {
            o.server.drop_db(&o.db).await?;
        }
        Ok(out?)
    }

    pub(crate) async fn inspect_target(&self, p: InspectParams) -> Result<Docs, PartError> {
        let docs = self
            .read(&p.target, async |c| {
                if p.keys.is_empty() {
                    Ok(Ok(rows::all_rows(c).await?.into_iter().collect()))
                } else {
                    rows::some_rows(c, &p.keys).await
                }
            })
            .await?;
        Ok(Docs {
            docs: docs.map_err(PartError::invalid)?,
        })
    }

    pub(crate) async fn diff_targets(&self, p: DiffParams) -> Result<Differing, PartError> {
        let a = self.read(&p.a, async |c| rows::comparable(c).await).await?;
        let b = self.read(&p.b, async |c| rows::comparable(c).await).await?;
        Ok(Differing::between(&a, &b))
    }
}
