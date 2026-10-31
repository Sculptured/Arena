//! Postgres for tests: the server URL, an admin handle, and a guard that drops the databases a
//! test made when it goes out of scope, also after a panic.
#![allow(dead_code)]

use std::{future::Future, sync::Mutex};

use arenad::pg::Admin;

pub fn pg_url() -> String {
    std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string())
}

pub fn admin() -> Admin {
    Admin::lazy(&pg_url())
}

/// Runs the future `f` makes on a thread and runtime of its own, so it works from inside a
/// test's runtime and from `Drop`, after a panic too.
pub fn on_own_runtime<T: Send, Fut: Future<Output = T>>(f: impl FnOnce() -> Fut + Send) -> T {
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(f())
        })
        .join()
        .unwrap()
    })
}

/// Drops `names` (a missing one is fine); failures are logged.
pub async fn drop_databases(names: &[String]) {
    let admin = admin();
    for n in names {
        if let Err(e) = admin.drop_db(n).await {
            eprintln!("test cleanup: could not drop {n}: {e:#}");
        }
    }
}

/// Drops every database it was told about when it goes out of scope.
#[derive(Default)]
pub struct Cleanup {
    names: Mutex<Vec<String>>,
}

impl Cleanup {
    pub fn new() -> Cleanup {
        Cleanup::default()
    }

    /// Drop this database at the end.
    pub fn add(&self, name: &str) {
        self.names.lock().unwrap().push(name.to_string());
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let names = std::mem::take(&mut *self.names.lock().unwrap());
        on_own_runtime(|| async move { drop_databases(&names).await });
    }
}
