//! Live test of the erpnext integration: the ERPNext stack (compose project `arena_erpnext`,
//! image `frappe/erpnext:v16.53.0`, never pulled) on 127.0.0.1:8180, with a MariaDB server
//! container and volume of the test's own. Everything it makes (the stack and its volumes, the
//! server, the saves and sites folder) is removed when the test ends, even when it fails.
//! Run: `cargo test -p arena-erpnext -- --ignored live_ --nocapture`.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use arena_erpnext::{Erpnext, StackConfig};
use arena_part::proto::*;
use arena_part::Part;
use serde_json::{json, Value};

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The test's stack, server and saves; all removed on drop.
struct Env {
    stack: StackConfig,
    saves: PathBuf,
}

impl Env {
    fn new() -> Env {
        let id = format!("t12-{}", std::process::id());
        let mut stack = StackConfig::default();
        stack.server.container = format!("arena-erpnext-db-{id}");
        stack.server.volume = format!("arena-erpnext-db-{id}");
        stack.server.port = free_port();
        let saves = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("erpnext-{id}"));
        std::fs::create_dir_all(&saves).unwrap();
        Env { stack, saves }
    }

    fn part(&self) -> Erpnext {
        Erpnext::new(self.stack.clone(), self.saves.join("erpnext"))
    }

    fn save_dir(&self, save: &str) -> String {
        self.saves
            .join("erpnext")
            .join(save)
            .to_string_lossy()
            .into()
    }

    /// The config that points a separate `arena-erpnext` process at the same server.
    fn check_config(&self) -> PathBuf {
        let s = &self.stack.server;
        let c = json!({"db_container": s.container, "db_volume": s.volume, "db_port": s.port});
        let path = self.saves.join("check.json");
        std::fs::write(&path, c.to_string()).unwrap();
        path
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // The server first: the project's network can't go while it is connected.
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", &self.stack.server.container])
            .output();
        let sites = self.saves.join("erpnext/.sites");
        let _ = Command::new("docker")
            .args(["compose", "-p", &self.stack.project, "-f"])
            .arg(&self.stack.compose_file)
            .args(["down", "-v"])
            .env("ARENA_ERPNEXT_SITES", &sites)
            .output();
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &self.stack.server.volume])
            .output();
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn start(instance: &str, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config: json!({}),
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn timed_start(p: &Erpnext, instance: &str, from: Option<&str>) {
    let t = Instant::now();
    p.start(start(instance, from)).await.unwrap();
    eprintln!("start {instance} from {from:?}: {:?}", t.elapsed());
}

async fn call(p: &Erpnext, inst: &str, name: &str, args: Value) -> ToolOutput {
    p.call_tool(CallToolParams {
        instance: inst.into(),
        name: name.into(),
        args,
        clock_ms: 0,
    })
    .await
    .unwrap()
}

/// A write tool on `i1` that must succeed and write `Item/ARENA-1`.
async fn write_item(p: &Erpnext, tool: &str, args: Value) {
    let out = call(p, "i1", tool, args).await;
    assert_eq!(out.error, None);
    assert_eq!(out.writes, ["Item/ARENA-1"]);
}

async fn item_name(p: &Erpnext, inst: &str, code: &str) -> String {
    let out = call(p, inst, "get_doc", json!({"doctype": "Item", "name": code})).await;
    assert_eq!(out.error, None);
    assert_eq!(out.reads, [format!("Item/{code}")]);
    out.result.unwrap()["item_name"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn save(p: &Erpnext, e: &Env, inst: &str, save: &str) -> Saved {
    let t = Instant::now();
    let s = p
        .save(SaveParams {
            instance: inst.into(),
            save: save.into(),
            dir: e.save_dir(save),
        })
        .await
        .unwrap();
    eprintln!(
        "save {save} of {inst}: {:?}, {} bytes",
        t.elapsed(),
        s.bytes
    );
    assert!(s.exact);
    s
}

async fn differing(p: &Erpnext, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

fn inst_t(s: &str) -> Target {
    Target::Instance { instance: s.into() }
}

fn save_t(s: &str) -> Target {
    Target::Save { save: s.into() }
}

fn df_root() -> String {
    let out = Command::new("df").args(["-h", "/"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.lines().last().unwrap_or_default().to_string()
}

/// Insert an Item, save, change it, restore the save into a new instance: the new instance has
/// the original. Then a fresh program (as after a respawn) restores the later save, and the
/// conformance check runs against the same stack.
#[tokio::test]
#[ignore]
async fn live_erpnext_roundtrip() {
    eprintln!("disk before: {}", df_root());
    let e = Env::new();
    let p = e.part();
    timed_start(&p, "i1", None).await; // the first start sets up the stack and the base site
    let item = json!({"doctype": "Item", "item_code": "ARENA-1", "item_name": "Original",
                      "item_group": "Products", "stock_uom": "Nos"});
    write_item(&p, "insert_doc", json!({ "doc": item })).await;
    save(&p, &e, "i1", "s1").await;
    let change = json!({"item_name": "Changed"});
    let args = json!({"doctype": "Item", "name": "ARENA-1", "fields": change});
    write_item(&p, "update_doc", args).await;
    timed_start(&p, "i2", Some("s1")).await;
    assert_eq!(item_name(&p, "i2", "ARENA-1").await, "Original");
    assert_eq!(item_name(&p, "i1", "ARENA-1").await, "Changed");
    check_reads_and_errors(&p).await;
    check_diff_and_inspect(&p).await;
    save(&p, &e, "i1", "s2").await;
    respawned_restore(&e).await;
    stop_and_drop(&p, &["i1", "i2"], &["s1", "s2"]).await;
    drop(p);
    conformance(&e);
    drop(e);
    eprintln!("disk after: {}", df_root());
    assert_nothing_left();
}

/// No container, volume or network of the stack or of an arena-erpnext server is left.
fn assert_nothing_left() {
    let list = |args: &[&str]| {
        let out = Command::new("docker").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let fmt = "{{.Name}}";
    let containers = list(&["ps", "-a", "--format", "{{.Names}}"]);
    let volumes = list(&["volume", "ls", "--format", fmt]);
    let networks = list(&["network", "ls", "--format", fmt]);
    let all = format!("{containers}{volumes}{networks}");
    let left: Vec<&str> = all
        .lines()
        .filter(|n| n.starts_with("arena_erpnext") || n.starts_with("arena-erpnext"))
        .collect();
    assert!(left.is_empty(), "left over: {left:?}");
}

async fn check_reads_and_errors(p: &Erpnext) {
    let out = call(p, "i2", "list_docs",
        json!({"doctype": "Item", "filters": {"item_code": "ARENA-1"}, "fields": ["name", "item_name"]})).await;
    assert_eq!(out.error, None);
    assert_eq!(
        out.result.unwrap(),
        json!([{"name": "ARENA-1", "item_name": "Original"}])
    );
    assert_eq!(out.reads, ["Item/ARENA-1"]);
    let out = call(
        p,
        "i2",
        "get_doc",
        json!({"doctype": "Item", "name": "NOPE"}),
    )
    .await;
    let err = out.error.expect("a missing doc is a tool error");
    assert!(err.contains("not found"), "{err}");
    let out = call(p, "i2", "insert_doc", json!({"doc": {"doctype": "Item"}})).await;
    assert!(out.error.is_some(), "an invalid doc is a tool error");
    let out = call(p, "i2", "get_doc", json!({"doctype": "Item"})).await;
    assert!(out.error.unwrap().contains("name"));
}

async fn check_diff_and_inspect(p: &Erpnext) {
    assert_eq!(
        differing(p, inst_t("i2"), save_t("s1")).await,
        Vec::<String>::new()
    );
    assert_eq!(
        differing(p, inst_t("i1"), save_t("s1")).await,
        ["Item/ARENA-1"]
    );
    let docs = p
        .inspect(InspectParams {
            target: save_t("s1"),
            keys: vec!["Item/ARENA-1".into(), "Item/NOPE".into()],
        })
        .await
        .unwrap()
        .docs;
    assert_eq!(docs["Item/ARENA-1"]["item_name"], "Original");
    assert_eq!(docs["Item/NOPE"], Value::Null);
}

/// A new program object (as after a crash) finds the base and the saves on disk.
async fn respawned_restore(e: &Env) {
    let p = e.part();
    timed_start(&p, "i3", Some("s2")).await;
    assert_eq!(item_name(&p, "i3", "ARENA-1").await, "Changed");
    p.stop(IdParams {
        instance: "i3".into(),
    })
    .await
    .unwrap();
}

async fn stop_and_drop(p: &Erpnext, instances: &[&str], saves: &[&str]) {
    let before = p.usage().await.unwrap().bytes;
    for i in instances {
        p.stop(IdParams {
            instance: (*i).into(),
        })
        .await
        .unwrap();
    }
    for s in saves {
        p.drop_save(SaveId { save: (*s).into() }).await.unwrap();
    }
    let after = p.usage().await.unwrap().bytes;
    eprintln!("usage with instances {before}, after stop and drop {after}");
    assert!(after < before);
}

fn conformance(e: &Env) {
    let a = arena();
    let ok = |dir: PathBuf, args: &[&str]| {
        let s = Command::new("cargo").args(args).current_dir(dir).status();
        assert!(s.unwrap().success(), "cargo {args:?}");
    };
    ok(a.clone(), &["build", "-p", "arena-erpnext"]);
    ok(
        a.join("core"),
        &["build", "-p", "parts", "--bin", "arena-part-check"],
    );
    let t = Instant::now();
    let out = Command::new(a.join("core/target/debug/arena-part-check"))
        .arg(a.join("integrations/erpnext/check"))
        .arg("--config")
        .arg(e.check_config())
        .env("ARENA_BIN_DIR", a.join("core/target/debug"))
        .env("ARENA_SAVES", &e.saves)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    eprintln!("conformance ({:?}):\n{stdout}", t.elapsed());
    assert!(
        out.status.success() && stdout.contains("\"passed\": true"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
