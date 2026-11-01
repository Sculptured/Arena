//! Live tests against a MariaDB server of their own (`mariadb:11.8`, never pulled). Each test
//! makes its own container and volume and removes both when it ends, even when it fails.
//! Run: `cargo test -p arena-mariadb -- --ignored live_ --nocapture`.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

use arena_mariadb::Mariadb;
use arena_part::helpers::mariadb::{Manifest, Server, ServerConfig};
use arena_part::proto::*;
use arena_part::Part;
use serde_json::{json, Value};

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A server container, volume and saves folder for one test; all removed on drop.
struct Env {
    container: String,
    volume: String,
    port: u16,
    saves: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Env {
        let id = format!("t11-{}-{tag}", std::process::id());
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let saves = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mariadb-{id}"));
        Env {
            container: format!("arena-mariadb-{id}"),
            volume: format!("arena-mariadb-{id}"),
            port,
            saves,
        }
    }

    fn config(&self, extra: Value) -> Value {
        let mut c = json!({
            "init_sql": "CREATE TABLE t(id int primary key, v text)",
            "container": self.container,
            "volume": self.volume,
            "port": self.port,
        });
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        c
    }

    fn part(&self) -> Mariadb {
        Mariadb::new(self.saves.join("mariadb")).with_sql_tools()
    }

    fn server(&self) -> Server {
        Server::new(ServerConfig {
            container: self.container.clone(),
            port: self.port,
            volume: self.volume.clone(),
            root_password: "arena".into(),
        })
        .unwrap()
    }

    fn save_dir(&self, save: &str) -> PathBuf {
        self.saves.join("mariadb").join(save)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", &self.container])
            .output();
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &self.volume])
            .output();
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

fn start(instance: &str, config: &Value, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config: config.clone(),
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn tool(p: &Mariadb, inst: &str, name: &str, sql: &str) -> ToolOutput {
    let out = p
        .call_tool(CallToolParams {
            instance: inst.into(),
            name: name.into(),
            args: json!({"sql": sql}),
            clock_ms: 0,
        })
        .await
        .unwrap();
    assert_eq!(out.error, None, "{sql}");
    out
}

async fn exec(p: &Mariadb, inst: &str, sql: &str) -> ToolOutput {
    tool(p, inst, "sql_execute", sql).await
}

async fn rows(p: &Mariadb, inst: &str) -> Value {
    let out = tool(p, inst, "sql_query", "SELECT id, v FROM t ORDER BY id").await;
    out.result.unwrap()
}

async fn save(p: &Mariadb, e: &Env, inst: &str, save: &str) -> Saved {
    let t = Instant::now();
    let s = p
        .save(SaveParams {
            instance: inst.into(),
            save: save.into(),
            dir: e.save_dir(save).to_string_lossy().into(),
        })
        .await
        .unwrap();
    eprintln!(
        "save {save} of {inst}: {:?}, {} bytes",
        t.elapsed(),
        s.bytes
    );
    s
}

async fn restore(p: &Mariadb, cfg: &Value, inst: &str, save: &str) {
    let t = Instant::now();
    p.start(start(inst, cfg, Some(save))).await.unwrap();
    eprintln!("restore {save} into {inst}: {:?}", t.elapsed());
}

async fn differing(p: &Mariadb, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

fn save_t(s: &str) -> Target {
    Target::Save { save: s.into() }
}

fn inst_t(s: &str) -> Target {
    Target::Instance { instance: s.into() }
}

#[tokio::test]
#[ignore]
async fn live_mariadb_roundtrip() {
    let e = Env::new("roundtrip");
    let cfg = e.config(json!({}));
    let p = e.part();
    let t = Instant::now();
    p.start(start("a", &cfg, None)).await.unwrap();
    eprintln!("first start (container up): {:?}", t.elapsed());
    let w = exec(&p, "a", "INSERT INTO t(id, v) VALUES (1, 'old')").await;
    assert_eq!(w.writes, ["t/1"]);
    assert_eq!(w.result, Some(json!({"rows_affected": 1})));
    assert!(save(&p, &e, "a", "s1").await.exact);
    let w = exec(&p, "a", "UPDATE t SET v = 'new' WHERE id = 1").await;
    assert_eq!(w.writes, ["t/1"]);
    save(&p, &e, "a", "s2").await;
    restore(&p, &cfg, "b", "s1").await;
    assert_eq!(rows(&p, "b").await, json!([{"id": 1, "v": "old"}]));
    restore(&p, &cfg, "c", "s2").await;
    assert_eq!(rows(&p, "c").await, json!([{"id": 1, "v": "new"}]));
    check_views(&p).await;
    check_lifecycle(&p, &e, &cfg).await;
}

/// inspect and diff over instances and saves.
async fn check_views(p: &Mariadb) {
    let docs = p
        .inspect(InspectParams {
            target: save_t("s1"),
            keys: vec!["t/1".into(), "t/2".into()],
        })
        .await
        .unwrap()
        .docs;
    assert_eq!(docs["t/1"], json!({"id": 1, "v": "old"}));
    assert_eq!(docs["t/2"], Value::Null);
    assert_eq!(differing(p, save_t("s1"), save_t("s2")).await, ["t/1"]);
    assert!(differing(p, inst_t("c"), save_t("s2")).await.is_empty());
    assert!(differing(p, save_t("s1"), save_t("s1")).await.is_empty());
}

/// describe, usage, drop_save, gc and stop.
async fn check_lifecycle(p: &Mariadb, e: &Env, cfg: &Value) {
    let d = p.describe().await.unwrap();
    assert_eq!(d.restore.exact, ExactKind::Save);
    assert!(!d.restore.approximate && !d.uses_clock);
    assert!(d.fingerprint["server_version"].is_string());
    assert!(p.usage().await.unwrap().bytes > 0);
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert!(!e.save_dir("s1").exists());
    let err = p.start(start("x", cfg, Some("s1"))).await.unwrap_err();
    assert_eq!(err.code, -32002);
    p.stop(IdParams {
        instance: "b".into(),
    })
    .await
    .unwrap();
    assert!(!e.server().exists("mi_b").await.unwrap());
    let r = p.gc(GcParams { keep: vec![] }).await.unwrap();
    assert_eq!(r.dropped, 1, "s2 dropped; live instances stay: {r:?}");
    assert!(e.server().exists("mi_a").await.unwrap());
}

#[tokio::test]
#[ignore]
async fn live_mariadb_restore_of_restore() {
    let e = Env::new("chain");
    let cfg = e.config(json!({}));
    let p = e.part();
    p.start(start("a", &cfg, None)).await.unwrap();
    exec(&p, "a", "INSERT INTO t VALUES (1, 'a')").await;
    save(&p, &e, "a", "s1").await;
    exec(&p, "a", "INSERT INTO t VALUES (99, 'after s1')").await;
    restore(&p, &cfg, "b", "s1").await;
    exec(&p, "b", "INSERT INTO t VALUES (2, 'b')").await;
    save(&p, &e, "b", "s2").await;
    let m = Manifest::read(&e.save_dir("s2")).unwrap();
    assert_eq!(m.root, None);
    let dbs: Vec<&str> = m.segments.iter().map(|s| s.db.as_str()).collect();
    assert_eq!(dbs, ["mi_a", "mi_b"], "the parent's history, then b's own");
    restore(&p, &cfg, "c", "s2").await;
    exec(&p, "c", "UPDATE t SET v = 'c' WHERE id = 2").await;
    exec(&p, "c", "INSERT INTO t VALUES (3, 'c')").await;
    save(&p, &e, "c", "s3").await;
    restore(&p, &cfg, "d", "s3").await;
    let want = json!([{"id": 1, "v": "a"}, {"id": 2, "v": "c"}, {"id": 3, "v": "c"}]);
    assert_eq!(rows(&p, "d").await, want);
    assert_eq!(
        rows(&p, "b").await,
        json!([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}])
    );
}

#[tokio::test]
#[ignore]
async fn live_mariadb_dump_threshold() {
    let e = Env::new("dump");
    let cfg = e.config(json!({"dump_after_bytes": 4000}));
    let p = e.part();
    p.start(start("a", &cfg, None)).await.unwrap();
    let big = "x".repeat(5000);
    exec(&p, "a", &format!("INSERT INTO t VALUES (1, '{big}')")).await;
    save(&p, &e, "a", "s1").await;
    let m1 = Manifest::read(&e.save_dir("s1")).unwrap();
    assert!(m1.root.is_some() && m1.segments.is_empty(), "{m1:?}");
    exec(&p, "a", "INSERT INTO t VALUES (2, 'small')").await;
    save(&p, &e, "a", "s2").await;
    let m2 = Manifest::read(&e.save_dir("s2")).unwrap();
    assert_eq!(m2.root, m1.root, "the dump is the new base");
    assert_eq!(m2.segments.len(), 1);
    rotate_binlogs(&e).await;
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert_eq!(blob_count(&e), 1, "s2 still needs the dump");
    check_purge_keeps_what_s2_needs(&p, &e, &m2).await;
    restore(&p, &cfg, "b", "s2").await;
    let r = rows(&p, "b").await;
    assert_eq!(r[0]["v"].as_str().unwrap().len(), 5000);
    assert_eq!(r[1], json!({"id": 2, "v": "small"}));
    check_dump_freed_last(&p, &e).await;
}

/// gc keeping s2 purges the binlog files before s2's oldest segment.
async fn check_purge_keeps_what_s2_needs(p: &Mariadb, e: &Env, m2: &Manifest) {
    p.gc(GcParams {
        keep: vec!["s2".into()],
    })
    .await
    .unwrap();
    let files = e.server().binlog_files().await.unwrap();
    assert_eq!(
        files[0].name, m2.segments[0].from.file,
        "older binlogs purged"
    );
}

/// The dump stays while a live instance starts from it, and goes once nothing does.
async fn check_dump_freed_last(p: &Mariadb, e: &Env) {
    p.drop_save(SaveId { save: "s2".into() }).await.unwrap();
    assert_eq!(
        blob_count(e),
        1,
        "live instances a and b start from the dump"
    );
    for i in ["a", "b"] {
        p.stop(IdParams { instance: i.into() }).await.unwrap();
    }
    p.gc(GcParams { keep: vec![] }).await.unwrap();
    assert_eq!(blob_count(e), 0);
}

async fn rotate_binlogs(e: &Env) {
    use arena_part::helpers::mariadb::mysql_async::prelude::Queryable;
    let mut c = e.server().conn(None).await.unwrap();
    for _ in 0..3 {
        c.query_drop("FLUSH BINARY LOGS").await.unwrap();
    }
}

fn blob_count(e: &Env) -> usize {
    std::fs::read_dir(e.saves.join("mariadb/.blobs"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|f| f.file_name().to_string_lossy().ends_with(".sql.gz"))
                .count()
        })
        .unwrap_or(0)
}

fn build_all() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let a = arena();
        let ok = |dir: PathBuf, args: &[&str]| {
            let s = Command::new("cargo").args(args).current_dir(dir).status();
            assert!(s.unwrap().success(), "cargo {args:?}");
        };
        ok(a.clone(), &["build", "-p", "arena-mariadb"]);
        ok(
            a.join("core"),
            &["build", "-p", "parts", "--bin", "arena-part-check"],
        );
    });
}

#[tokio::test]
#[ignore]
async fn live_mariadb_conformance() {
    build_all();
    let e = Env::new("check");
    let a = arena();
    std::fs::create_dir_all(&e.saves).unwrap();
    let cfgfile = e.saves.join("check.json");
    std::fs::write(&cfgfile, e.config(json!({})).to_string()).unwrap();
    let saves = e.saves.join("saves");
    let out = Command::new(a.join("core/target/debug/arena-part-check"))
        .arg(a.join("integrations/mariadb/check"))
        .arg("--config")
        .arg(&cfgfile)
        .env("ARENA_BIN_DIR", a.join("core/target/debug"))
        .env("ARENA_SAVES", &saves)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    eprintln!("{stdout}");
    assert!(
        out.status.success() && stdout.contains("\"passed\": true"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
