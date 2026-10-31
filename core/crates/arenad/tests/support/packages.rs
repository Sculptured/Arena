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

use crate::pg::{drop_databases, on_own_runtime, pg_url};

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

/// `cargo build -p arena-postgres -p arena-retail` and the kit's `counter` example (debug),
/// once per test binary.
pub fn build() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let builds: [&[&str]; 2] = [
            &["build", "-q", "-p", "arena-postgres", "-p", "arena-retail"],
            &["build", "-q", "--example", "counter", "-p", "arena-part"],
        ];
        for args in builds {
            let st = Command::new("cargo")
                .args(args)
                .current_dir(arena())
                .status()
                .unwrap();
            assert!(st.success(), "building the package binaries failed");
        }
    });
}

/// A hub over the real packages with no Postgres namespace, so the integration accepts the
/// database names of runs recorded before integrations. Saves go under `saves`. Only for tests
/// that make uniquely named databases and drop them themselves; never run `gc` on it.
pub fn plain_hub(saves: &Path) -> Arc<Hub> {
    build();
    let registry = Registry::scan(&package_roots()).unwrap();
    let hub = Hub::new(registry, bin_dir(), saves.to_path_buf())
        .with_env(vec![("ARENA_PG_URL".into(), pg_url())]);
    Arc::new(hub)
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
        let hub = hub_on(&ns, &saves, extra);
        TestHub { hub, ns, saves }
    }

    /// Another hub on this one's namespace and save folder, over the real packages plus `extra`
    /// (as if arenad restarted with other packages installed).
    pub fn sibling(&self, extra: &[PathBuf]) -> Arc<Hub> {
        hub_on(&self.ns, &self.saves, extra)
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

/// A hub over the real packages plus `extra`, in Postgres namespace `ns`, saving under `saves`.
fn hub_on(ns: &str, saves: &Path, extra: &[PathBuf]) -> Arc<Hub> {
    let mut roots = package_roots();
    roots.extend(extra.iter().cloned());
    let url = pg_url();
    let sep = if url.contains('?') { '&' } else { '?' };
    let hub = Hub::new(
        Registry::scan(&roots).unwrap(),
        bin_dir(),
        saves.to_path_buf(),
    )
    .with_env(vec![(
        "ARENA_PG_URL".into(),
        format!("{url}{sep}arena_ns={ns}"),
    )]);
    Arc::new(hub)
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
    drop(client);
    let _ = task.await;
    drop_databases(&names).await;
    Ok(())
}

impl Drop for TestHub {
    fn drop(&mut self) {
        let ns = self.ns.clone();
        if let Err(e) = on_own_runtime(|| async move { drop_namespace(&ns).await }) {
            eprintln!("test cleanup failed: {e:#}");
        }
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

/// The folder `package_root(tag, ..)` makes (for files the toy programs write, like `--log`).
pub fn toy_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("arenad-toy-{tag}-{}", std::process::id()))
}

/// A package for a test root: its name, kind and `run.command`.
pub struct Package {
    pub name: String,
    pub kind: &'static str,
    pub command: Vec<String>,
}

/// A toy package (`tests/fixtures/toy.py`), run as `python3 toy.py <args..>`.
pub fn toy(name: &str, kind: &'static str, args: &[&str]) -> Package {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy.py");
    let mut command = vec!["python3".to_string(), script.to_string_lossy().into_owned()];
    command.extend(args.iter().map(|a| a.to_string()));
    Package {
        name: name.to_string(),
        kind,
        command,
    }
}

/// An integration running the kit's `counter` example with `args` (e.g. `--flavour replay_only`).
pub fn counter(name: &str, args: &[&str]) -> Package {
    let mut command = vec!["${ARENA_BIN_DIR}/examples/counter".to_string()];
    command.extend(args.iter().map(|a| a.to_string()));
    Package {
        name: name.to_string(),
        kind: "integration",
        command,
    }
}

/// `packages` in a fresh folder. Pass `.0` to `TestHub::new` as an extra root.
pub fn package_root(tag: &str, packages: &[Package]) -> ToyRoot {
    let root = toy_dir(tag);
    let _ = std::fs::remove_dir_all(&root);
    for p in packages {
        let d = root.join(&p.name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("arena-package.toml"), manifest(p)).unwrap();
    }
    ToyRoot(root)
}

fn manifest(p: &Package) -> String {
    let command: Vec<String> = p.command.iter().map(|a| format!("{a:?}")).collect();
    let (kind, name, command) = (p.kind, &p.name, command.join(", "));
    format!("kind = \"{kind}\"\nname = \"{name}\"\nversion = \"0.1.0\"\nprotocol = 1\nrun = {{ command = [{command}] }}\n")
}

/// A toy integration `toy-<tag>` started with `part_args`, and a toy benchmark world of it.
pub fn toy_world(tag: &str, part_args: &[&str], bench_args: &[&str]) -> ToyRoot {
    let part = format!("toy-{tag}");
    let mut part_argv = vec!["part"];
    part_argv.extend_from_slice(part_args);
    let mut bench_argv = vec!["bench", "--integration", part.as_str()];
    bench_argv.extend_from_slice(bench_args);
    package_root(
        tag,
        &[
            toy(&part, "integration", &part_argv),
            toy("toybench", "benchmark", &bench_argv),
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
