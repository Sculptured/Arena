//! The real packages (`arena-postgres`, `arena-retail`, built once per test binary) behind a hub
//! of the test's own: its own Postgres namespace (`?arena_ns=` on `ARENA_PG_URL`) and its own save
//! folder, both removed when the `TestHub` drops, so tests never touch each other's databases or
//! the live arenad's.
#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

use parts::{Hub, Registry};

/// The arena folder (this crate is core/crates/arenad).
pub fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// `core/target/debug`, where `build()` puts the package binaries.
pub fn bin_dir() -> PathBuf {
    arena().join("core/target/debug")
}

/// The roots `ARENA_PACKAGES` would name: `<arena>/integrations` and `<arena>/benchmarks`.
pub fn package_roots() -> Vec<PathBuf> {
    vec![arena().join("integrations"), arena().join("benchmarks")]
}

/// `cargo build -p arena-postgres -p arena-retail` (debug), once per test binary.
pub fn build() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let st = Command::new("cargo")
            .args(["build", "-q", "-p", "arena-postgres", "-p", "arena-retail"])
            .current_dir(arena())
            .status()
            .unwrap();
        assert!(st.success(), "building the package binaries failed");
    });
}

pub fn pg_url() -> String {
    std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string())
}

/// A hub over the real packages plus `extra` roots, with its own namespace and save folder.
pub struct TestHub {
    pub hub: Arc<Hub>,
    pub ns: String,
    pub saves: PathBuf,
}

impl TestHub {
    pub fn new(extra: &[PathBuf]) -> TestHub {
        build();
        static N: AtomicU64 = AtomicU64::new(0);
        let ns = format!(
            "t7{}n{}_",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        let saves = std::env::temp_dir().join(format!("arenad-saves-{ns}"));
        let _ = std::fs::remove_dir_all(&saves);
        std::fs::create_dir_all(&saves).unwrap();
        let mut roots = package_roots();
        roots.extend(extra.iter().cloned());
        let url = pg_url();
        let sep = if url.contains('?') { '&' } else { '?' };
        let hub = Hub::new(Registry::scan(&roots).unwrap(), bin_dir(), saves.clone()).with_env(
            vec![("ARENA_PG_URL".into(), format!("{url}{sep}arena_ns={ns}"))],
        );
        TestHub {
            hub: Arc::new(hub),
            ns,
            saves,
        }
    }

    /// `bench`'s world for `branch`, started from the base save with seed 1.
    pub async fn start_world(
        &self,
        bench: &str,
        branch: &str,
    ) -> std::sync::Arc<arenad::world::World> {
        let described = self.hub.bench_describe(bench).await.unwrap();
        let from = arenad::world::WorldFrom::Base;
        let world =
            arenad::world::World::start(self.hub.clone(), &described, branch, 1, from).await;
        std::sync::Arc::new(world.unwrap())
    }

    /// The `inspect` docs of `keys` in a save of the postgres integration.
    pub async fn inspect_save(&self, save: &str, keys: &[&str]) -> serde_json::Value {
        let c = self.hub.client("postgres").await.unwrap();
        let r: serde_json::Value = c
            .call(
                "inspect",
                serde_json::json!({"target": {"save": save}, "keys": keys}),
            )
            .await
            .unwrap();
        r["docs"].clone()
    }

    /// The keys that differ between two postgres targets (`{"save": ..}` or `{"instance": ..}`).
    pub async fn diff(&self, a: serde_json::Value, b: serde_json::Value) -> Vec<String> {
        let c = self.hub.client("postgres").await.unwrap();
        let r: serde_json::Value = c
            .call("diff", serde_json::json!({"a": a, "b": b}))
            .await
            .unwrap();
        serde_json::from_value(r["differing"].clone()).unwrap()
    }
}

/// Drops every `pi_<ns>*` and `ps_<ns>*` database (this hub's instances and saves).
async fn drop_namespace(ns: &str) -> anyhow::Result<()> {
    let (client, conn) = tokio_postgres::connect(&pg_url(), tokio_postgres::NoTls).await?;
    let task = tokio::spawn(conn);
    let names: Vec<String> = client
        .query("SELECT datname::text FROM pg_database", &[])
        .await?
        .iter()
        .map(|r| r.get(0))
        .filter(|n: &String| {
            n.starts_with(&format!("pi_{ns}")) || n.starts_with(&format!("ps_{ns}"))
        })
        .collect();
    for n in names {
        if let Err(e) = client
            .batch_execute(&format!("DROP DATABASE IF EXISTS \"{n}\" WITH (FORCE)"))
            .await
        {
            eprintln!("test cleanup: could not drop {n}: {e}");
        }
    }
    drop(client);
    let _ = task.await;
    Ok(())
}

impl Drop for TestHub {
    fn drop(&mut self) {
        let ns = self.ns.clone();
        // Own thread and runtime: works from inside a test's runtime and after a panic.
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                if let Err(e) = rt.block_on(drop_namespace(&ns)) {
                    eprintln!("test cleanup failed: {e:#}");
                }
            });
        });
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

/// A folder of toy packages, removed on drop.
pub struct ToyRoot(pub PathBuf);

impl Drop for ToyRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Toy packages (`tests/fixtures/toy.py`) in a fresh folder: `(name, kind, args)` each, run as
/// `python3 toy.py <args..>`. Pass `.0` to `TestHub::new` as an extra root.
pub fn toy_root(tag: &str, packages: &[(&str, &str, &[&str])]) -> ToyRoot {
    let root = std::env::temp_dir().join(format!("arenad-toy-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (name, kind, args) in packages {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("arena-package.toml"), toy_manifest(name, kind, args)).unwrap();
    }
    ToyRoot(root)
}

fn toy_manifest(name: &str, kind: &str, args: &[&str]) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy.py");
    let mut argv = vec![
        format!("{:?}", "python3"),
        format!("{:?}", script.to_string_lossy()),
    ];
    argv.extend(args.iter().map(|a| format!("{a:?}")));
    let command = argv.join(", ");
    format!("kind = \"{kind}\"\nname = \"{name}\"\nversion = \"0.1.0\"\nprotocol = 1\nrun = {{ command = [{command}] }}\n")
}

/// A toy integration `toy-<tag>` started with `part_args`, and a toy benchmark world of it.
pub fn toy_world(tag: &str, part_args: &[&str], bench_args: &[&str]) -> ToyRoot {
    let part = format!("toy-{tag}");
    let mut part_argv = vec!["part"];
    part_argv.extend_from_slice(part_args);
    let mut bench_argv = vec!["bench", "--integration", part.as_str()];
    bench_argv.extend_from_slice(bench_args);
    toy_root(
        tag,
        &[
            (&part, "integration", &part_argv),
            ("toybench", "benchmark", &bench_argv),
        ],
    )
}

/// The `arguments` of the gold action `name` of a retail task (from `data/retail/tasks.json`).
pub fn gold_action_args(task: &str, name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(arena().join("data/retail/tasks.json")).unwrap();
    let tasks: serde_json::Value = serde_json::from_str(&text).unwrap();
    let t = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task)
        .unwrap();
    let a = t["evaluation_criteria"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap();
    a["arguments"].clone()
}
