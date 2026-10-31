use parts::{CallError, Hub, Manifest, Registry};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The arena folder (this crate is core/crates/parts).
fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

fn bin_dir() -> PathBuf {
    arena().join("core/target/debug")
}

fn build_counter() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let st = Command::new("cargo")
            .args(["build", "--example", "counter", "-p", "arena-part"])
            .current_dir(arena())
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
        bin_dir(),
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
    assert_eq!(hub.package_saves_bytes("counter"), hub.saves_bytes());
    assert_eq!(hub.save_dirs("counter"), ["s1"]);
    // A dropped save's folder goes; bad ids and unknown packages are ignored.
    let dir = hub.save_dir("counter", "s1");
    hub.remove_save_dir("counter", "../counter");
    hub.remove_save_dir("nope", "s1");
    assert!(dir.exists());
    hub.remove_save_dir("counter", "s1");
    assert!(!dir.exists() && hub.saves_bytes() == 0 && hub.save_dirs("counter").is_empty());
    drop(hub);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn hub_spawns_python_counter() {
    let saves = temp("py-saves");
    // kits/python/examples holds the manifest, so kits/python is the package root
    let hub = hub_for(&[arena().join("kits/python")], &saves);
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

#[tokio::test]
async fn hub_env_overrides_reach_command_programs() {
    let (root, saves) = (temp("envover"), temp("envover-saves"));
    write_pkg(
        &root,
        "envo",
        r#"["sh", "-c", "echo ARENA_PG_URL=$ARENA_PG_URL EXTRA=$EXTRA >&2; exit 1"]"#,
    );
    let hub = hub_for(std::slice::from_ref(&root), &saves).with_env(vec![
        ("ARENA_PG_URL".into(), "postgres://x/y?arena_ns=t_".into()),
        ("EXTRA".into(), "yes".into()),
    ]);
    let c = hub.client("envo").await.unwrap();
    match c.call::<Value>("describe", json!({})).await {
        Err(CallError::Stopped(m)) => {
            assert!(
                m.contains("ARENA_PG_URL=postgres://x/y?arena_ns=t_ EXTRA=yes"),
                "{m}"
            );
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

fn slow_part(root: &Path) {
    write_pkg(
        root,
        "slow",
        r#"["sh", "-c", "while read l; do sleep 0.3; id=$(echo \"$l\" | sed 's/.*\"id\":\\([0-9]*\\).*/\\1/'); echo \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"id\\\":$id,\\\"result\\\":{}}\"; done"]"#,
    );
}

#[tokio::test]
async fn dropped_call_leaves_nothing_behind() {
    let (root, saves) = (temp("slow"), temp("slow-saves"));
    slow_part(&root);
    let hub = hub_for(std::slice::from_ref(&root), &saves);
    let c = hub.client("slow").await.unwrap();
    let r = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        c.call::<Value>("usage", json!({})),
    )
    .await;
    assert!(r.is_err(), "the call should still be waiting");
    assert_eq!(c.pending_len(), 0);
    let _: Value = c.call("usage", json!({})).await.unwrap();
    assert_eq!(c.pending_len(), 0);
    drop((c, hub));
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

#[tokio::test]
async fn timeout_is_reported_and_cleaned() {
    let (root, saves) = (temp("to"), temp("to-saves"));
    slow_part(&root);
    let m = Manifest::load(&root.join("slow")).unwrap();
    let c = parts::Client::spawn(&m, &bin_dir(), &saves)
        .await
        .unwrap()
        .with_timeout(std::time::Duration::from_millis(20));
    assert!(matches!(
        c.call::<Value>("usage", json!({})).await,
        Err(CallError::Timeout)
    ));
    assert_eq!(c.pending_len(), 0);
    drop(c);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&saves).unwrap();
}

fn group_alive(pgid: u32) -> bool {
    Command::new("pgrep")
        .args(["-g", &pgid.to_string()])
        .output()
        .unwrap()
        .status
        .success()
}

#[tokio::test]
async fn killed_uv_python_respawns_and_leaves_no_process() {
    let saves = temp("uvkill-saves");
    let hub = hub_for(&[arena().join("kits/python")], &saves);
    let c = hub.client("counter-py").await.unwrap();
    let _: Value = c.call("usage", json!({})).await.unwrap();
    Command::new("kill")
        .args(["-9", &c.pid().to_string()])
        .status()
        .unwrap();
    let start = std::time::Instant::now();
    while !c.is_stopped() {
        assert!(
            start.elapsed().as_secs() < 5,
            "client never noticed the exit"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let c2 = hub.client("counter-py").await.unwrap();
    assert_ne!(c.pid(), c2.pid());
    let _: Value = c2.call("usage", json!({})).await.unwrap();
    assert!(!group_alive(c.pid()), "old process group still running");
    let pgid = c2.pid();
    drop((c, c2, hub));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!group_alive(pgid), "dropping the client left processes");
    std::fs::remove_dir_all(&saves).unwrap();
}
