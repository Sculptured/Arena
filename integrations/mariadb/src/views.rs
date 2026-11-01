//! `inspect` and `diff`: an instance is read in place; a save is rebuilt into a scratch database
//! `mx_<pid>_<n>` (unlogged) for the read and dropped after.
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use arena_part::helpers::mariadb::mysql_async::Conn;
use arena_part::helpers::mariadb::Server;
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
    async fn open(&self, t: &Target) -> Result<Opened, PartError> {
        let save = match t {
            Target::Instance { instance } => {
                let i = self.instance(instance)?;
                return Ok(Opened {
                    server: i.server,
                    db: i.db,
                    view: false,
                });
            }
            Target::Save { save } => save,
        };
        let m = self.manifest(save)?;
        let server = self.servers.for_ref(&m.server)?;
        let db = view_name();
        self.views.lock().expect("views lock").insert(db.clone());
        if let Err(e) = server.restore(&m, &self.blobs, &db).await {
            self.views.lock().expect("views lock").remove(&db);
            return Err(PartError::internal(format!("reading save {save}: {e:#}")));
        }
        Ok(Opened {
            server,
            db,
            view: true,
        })
    }

    async fn close(&self, o: Opened) {
        if o.view {
            let _ = o.server.drop_db(&o.db).await;
            self.views.lock().expect("views lock").remove(&o.db);
        }
    }

    /// Runs `f` on a connection to the target's database.
    async fn read<T>(
        &self,
        t: &Target,
        f: impl AsyncFnOnce(&mut Conn) -> anyhow::Result<T>,
    ) -> Result<T, PartError> {
        let o = self.open(t).await?;
        let out = o.server.with_db(&o.db, f).await;
        self.close(o).await;
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
        let keys: HashSet<&String> = a.keys().chain(b.keys()).collect();
        let mut differing: Vec<String> = keys
            .into_iter()
            .filter(|k| a.get(*k) != b.get(*k))
            .cloned()
            .collect();
        differing.sort();
        Ok(Differing { differing })
    }

    /// The scratch databases being read now (gc leaves them alone).
    pub(crate) fn open_views(&self) -> HashSet<String> {
        self.views.lock().expect("views lock").clone()
    }
}
