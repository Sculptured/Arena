use parts::{CallError, Hub, Manifest, Registry};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const ARENA: &str = "/home/alexsky2/MLSys/Sculptured/arena";
const BIN_DIR: &str = "/home/alexsky2/MLSys/Sculptured/arena/core/target/debug";

fn build_counter() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let st = Command::new("cargo")
            .args(["build", "--example", "counter", "-p", "arena-part"])
            .current_dir(ARENA)
            .status()
            .unwrap();
        assert!(st.success());
    });
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("parts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_pkg(root: &Path, name: &str, command: &str) {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("arena-package.toml"),
        format!(
            "kind = \"integration\"\nname = \"{name}\"\nversion = \"0.1.0\"\nprotocol = 1\nrun = {{ command = {command} }}\n"
        ),
    )
    .unwrap();
}

fn hub_for(roots: &[PathBuf], saves: &Path) -> Hub {
    Hub::new(
        Registry::scan(roots).unwrap(),
        BIN_DIR.into(),
        saves.to_path_buf(),
    )
}

async fn exercise(hub: &Hub, name: &str) {
    let d = hub.describe(name).await.unwrap();
    assert_eq!(d.protocol, 1);
    let c = hub.client(name).await.unwrap();
    let _: Value = c
        .call(
            "start",
            json!({"instance": "i1", "config": {}, "seed": 1, "clock_ms": 0}),
        )
        .await
        .unwrap();
    let out: Value = c
        .call(
            "call_tool",
            json!({"instance": "i1", "name": "add", "args": {"n": 5}, "clock_ms": 0}),
        )
        .await
        .unwrap();
    assert_eq!(out["result"]["value"], 5);
    let dir = hub.save_dir(name, "s1");
    assert_eq!(dir, hub.save_dir(name, "s1"));
    assert!(dir.ends_with(format!("{name}/s1")));
    let saved: Value = c
        .call("save", json!({"instance": "i1", "save": "s1", "dir": dir}))
        .await
        .unwrap();
    assert_eq!(saved["exact"], true);
}

#[test]
fn manifest_rejects_bad_name() {
    let root = temp("badname");
    write_pkg(&root, "bad", r#"["x"]"#);
    let f = root.join("bad/arena-package.toml");
    let t = std::fs::read_to_string(&f)
        .unwrap()
        .replace("name = \"bad\"", "name = \"Bad_Name\"");
    std::fs::write(&f, &t).unwrap();
    assert!(Manifest::load(&root.join("bad")).is_err());
    std::fs::write(
        &f,
        t.replace("Bad_Name", "good")
            .replace("protocol = 1", "protocol = 2"),
    )
    .unwrap();
    assert!(Manifest::load(&root.join("bad")).is_err());
    std::fs::write(&f, t.replace("Bad_Name", "good")).unwrap();
    assert_eq!(Manifest::load(&root.join("bad")).unwrap().name, "good");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn registry_rejects_duplicates() {
    let (a, b) = (temp("dup-a"), temp("dup-b"));
    write_pkg(&a, "same", r#"["x"]"#);
    write_pkg(&b, "same", r#"["x"]"#);
    assert!(Registry::scan(&[a.clone(), b.clone()]).is_err());
    let r = Registry::scan(&[a.clone(), PathBuf::from("/nonexistent-root")]).unwrap();
    assert!(r.get("same").is_some());
    assert_eq!(r.integrations().count(), 1);
    assert_eq!(r.benchmarks().count(), 0);
    std::fs::remove_dir_all(&a).unwrap();
    std::fs::remove_dir_all(&b).unwrap();
}

#[tokio::test]
async fn hub_spawns_rust_counter_and_saves() {
    build_counter();
    let (root, saves) = (temp("rust"), temp("rust-saves"));
    write_pkg(&root, "counter", r#"["${ARENA_BIN_DIR}/examples/counter"]"#);
    let hub = hub_for(std::slice::from_ref(&root), &saves);
    exercise(&hub, "counter").await;
    assert!(hub.saves_bytes() > 0);
    drop(hub);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn hub_spawns_python_counter() {
    let saves = temp("py-saves");
    // kits/python/examples holds the manifest, so kits/python is the package root
    let hub = hub_for(&[PathBuf::from(ARENA).join("kits/python")], &saves);
    exercise(&hub, "counter-py").await;
    drop(hub);
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn hub_respawns_after_crash() {
    build_counter();
    let (root, saves) = (temp("crash"), temp("crash-saves"));
    write_pkg(&root, "counter", r#"["${ARENA_BIN_DIR}/examples/counter"]"#);
    let hub = hub_for(std::slice::from_ref(&root), &saves);
    let c = hub.client("counter").await.unwrap();
    let _: Value = c.call("usage", json!({})).await.unwrap();
    Command::new("kill")
        .args(["-9", &c.pid().to_string()])
        .status()
        .unwrap();
    let r: Result<Value, CallError> = c.call("usage", json!({})).await;
    assert!(matches!(r, Err(CallError::Stopped(_))));
    let c2 = hub.client("counter").await.unwrap();
    assert_ne!(c.pid(), c2.pid());
    let _: Value = c2.call("usage", json!({})).await.unwrap();
    assert!(hub.describe("counter").await.is_ok());
    drop((c, c2, hub));
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn stopped_error_carries_stderr() {
    let (root, saves) = (temp("stderr"), temp("stderr-saves"));
    write_pkg(
        &root,
        "boomer",
        r#"["sh", "-c", "echo boom happened >&2; exit 3"]"#,
    );
    let hub = hub_for(std::slice::from_ref(&root), &saves);
    let c = hub.client("boomer").await.unwrap();
    match c.call::<Value>("describe", json!({})).await {
        Err(CallError::Stopped(m)) => assert!(m.contains("boom happened"), "{m}"),
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn command_mode_clears_environment() {
    let (root, saves) = (temp("env"), temp("env-saves"));
    write_pkg(
        &root,
        "envy",
        r#"["sh", "-c", "echo ARENA_SAVES=$ARENA_SAVES FOO=$FOO >&2; exit 1"]"#,
    );
    std::env::set_var("FOO", "leaked");
    let hub = hub_for(std::slice::from_ref(&root), &saves);
    let c = hub.client("envy").await.unwrap();
    match c.call::<Value>("describe", json!({})).await {
        Err(CallError::Stopped(m)) => {
            assert!(
                m.contains(&format!("ARENA_SAVES={}", saves.display())),
                "{m}"
            );
            assert!(!m.contains("leaked"), "{m}");
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}
