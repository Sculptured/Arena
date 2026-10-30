use crate::pg::Admin;

/// Test helper: connects using `ARENA_PG_URL` (default: the compose Postgres on 5544).
pub fn admin() -> Admin {
    let url = std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string());
    Admin::lazy(&url)
}

/// Test guard: drops every database it was told about when it goes out of scope, including
/// when the test panics. Dropping runs on its own thread with its own runtime, so it works
/// from inside any runtime and after the test's runtime state is gone.
///
/// What it removes: names added with `add`; if `meta` names an arena metadata database, every
/// `branch_*` database whose name contains one of its branch ids (branches, fork and row-check
/// temp copies, gold copies), every snapshot its tool-call events name, and the metadata
/// database itself.
#[derive(Default)]
pub struct Cleanup {
    names: std::sync::Mutex<Vec<String>>,
    tokens: std::sync::Mutex<Vec<String>>,
    meta: std::sync::Mutex<Option<String>>,
}

impl Cleanup {
    pub fn new() -> Cleanup {
        Cleanup::default()
    }

    /// Drop this database at the end.
    pub fn add(&self, name: &str) {
        self.names.lock().unwrap().push(name.to_string());
    }

    /// Drop every `branch_*` database whose name contains `token` at the end.
    pub fn branches_containing(&self, token: &str) {
        self.tokens.lock().unwrap().push(token.to_string());
    }

    /// The arena metadata database to sweep (and drop) at the end.
    pub fn meta(&self, name: &str) {
        *self.meta.lock().unwrap() = Some(name.to_string());
    }
}

async fn sweep(c: &Cleanup) -> anyhow::Result<()> {
    let admin = admin();
    let mut drop_list: Vec<String> = c.names.lock().unwrap().clone();
    let mut tokens: Vec<String> = c.tokens.lock().unwrap().clone();
    let meta = c.meta.lock().unwrap().clone();
    if let Some(meta) = &meta {
        if admin.exists(meta).await? {
            let (ids, snaps) = admin
                .with_db(meta, async |cl| {
                    let ids: Vec<String> =
                        cl.query("SELECT id FROM branches", &[]).await?.iter().map(|r| r.get(0)).collect();
                    let snaps: Vec<String> = cl
                        .query(
                            "SELECT DISTINCT payload->>'snapshot' FROM events WHERE kind = 'tool_call' AND payload->>'snapshot' IS NOT NULL",
                            &[],
                        )
                        .await?
                        .iter()
                        .map(|r| r.get(0))
                        .collect();
                    Ok((ids, snaps))
                })
                .await?;
            tokens.extend(ids);
            drop_list.extend(snaps);
        }
    }
    if !tokens.is_empty() {
        for (name, _) in admin.list_prefix("branch_").await? {
            if tokens.iter().any(|t| name.contains(t.as_str())) {
                drop_list.push(name);
            }
        }
    }
    if let Some(meta) = meta {
        drop_list.push(meta);
    }
    for name in drop_list {
        if let Err(e) = admin.drop_db(&name).await {
            eprintln!("cleanup: could not drop {name}: {e:#}");
        }
    }
    Ok(())
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                if let Err(e) = rt.block_on(sweep(self)) {
                    eprintln!("cleanup failed: {e:#}");
                }
            });
        });
    }
}
