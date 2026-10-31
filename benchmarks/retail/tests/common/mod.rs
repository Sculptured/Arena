#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use arena_part::helpers::pg::Templates;
use arena_retail::Retail;
use serde_json::{json, Map, Value};

static N: AtomicU32 = AtomicU32::new(0);

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/retail")
}

pub fn pg_url() -> String {
    std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".into())
}

pub fn retail() -> Retail {
    Retail::new(&data_dir()).unwrap()
}

pub fn tasks() -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(data_dir().join("tasks.json")).unwrap()).unwrap()
}

pub fn fixture() -> Map<String, Value> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gold_changes.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// A scratch database that is dropped when the guard goes, even when the test panics.
pub struct TestDb {
    pub name: String,
    pub url: String,
}

impl TestDb {
    pub async fn empty(tag: &str) -> TestDb {
        let name = format!(
            "t6_{}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst),
            tag
        );
        let t = Templates::new(&pg_url());
        t.drop(&name).await.unwrap();
        t.create_empty(&name).await.unwrap();
        TestDb {
            url: t.db_url(&name),
            name,
        }
    }

    /// A database seeded through the benchmark's own `seed`.
    pub async fn seeded(tag: &str) -> TestDb {
        use arena_part::proto::SeedParams;
        use arena_part::Benchmark;
        let db = TestDb::empty(tag).await;
        retail()
            .seed(SeedParams {
                connections: conns(&db.url),
            })
            .await
            .unwrap();
        db
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let _ = Templates::new(&pg_url()).drop(&name).await;
            });
        })
        .join();
    }
}

pub fn conns(url: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("retail".into(), json!({"url": url}));
    m
}
