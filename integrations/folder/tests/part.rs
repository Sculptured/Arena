use arena_folder::Folder;
use arena_part::proto::*;
use arena_part::Part;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A temporary saves folder, removed on drop even when the test fails.
struct Saves(PathBuf);

impl Saves {
    fn new(tag: &str) -> Saves {
        let p = std::env::temp_dir().join(format!("arena_folder_{}_{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Saves(p)
    }
    fn part(&self) -> Folder {
        Folder::new(self.0.join("folder"))
    }
    fn save_dir(&self, save: &str) -> PathBuf {
        let d = self.0.join("folder").join(save);
        fs::create_dir_all(&d).unwrap();
        d
    }
}

impl Drop for Saves {
    fn drop(&mut self) {
        let _ = arena_part::helpers::remove_tree(&self.0);
    }
}

fn start(instance: &str, config: Value, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config,
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn tool(p: &Folder, inst: &str, name: &str, args: Value) -> ToolOutput {
    p.call_tool(CallToolParams {
        instance: inst.into(),
        name: name.into(),
        args,
        clock_ms: 0,
    })
    .await
    .unwrap()
}

async fn write(p: &Folder, inst: &str, path: &str, content: &str) -> ToolOutput {
    tool(
        p,
        inst,
        "write_file",
        json!({"path": path, "content": content}),
    )
    .await
}

async fn save(s: &Saves, p: &Folder, inst: &str, id: &str) -> Saved {
    let dir = s.save_dir(id);
    p.save(SaveParams {
        instance: inst.into(),
        save: id.into(),
        dir: dir.to_string_lossy().into(),
    })
    .await
    .unwrap()
}

fn target_i(i: &str) -> Target {
    Target::Instance { instance: i.into() }
}

fn target_s(s: &str) -> Target {
    Target::Save { save: s.into() }
}

#[tokio::test]
async fn write_outside_root_refused() {
    let s = Saves::new("outside");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    for bad in ["../x", "a/../../x", "/etc/arena_x", ""] {
        let out = write(&p, "a", bad, "no").await;
        assert!(out.error.is_some(), "{bad:?} should be refused: {out:?}");
        assert!(out.writes.is_empty());
    }
    assert!(!s.0.join("folder/x").exists() && !s.0.join("x").exists());
    let out = tool(&p, "a", "read_file", json!({"path": "../x"})).await;
    assert!(out.error.is_some());
}

#[tokio::test]
async fn symlink_escape_refused() {
    let s = Saves::new("symlink");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    let live = s.0.join("folder/.live/a");
    let outside = s.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, live.join("link")).unwrap();
    std::os::unix::fs::symlink(outside.join("dangling"), live.join("dangling")).unwrap();
    for bad in ["link/f", "dangling"] {
        let out = write(&p, "a", bad, "no").await;
        assert!(out.error.is_some(), "{bad}: {out:?}");
    }
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[tokio::test]
async fn tools_write_read_list() {
    let s = Saves::new("tools");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    let w = write(&p, "a", "notes/./x.txt", "hello").await;
    assert_eq!(w.error, None);
    assert_eq!(w.writes, vec!["file/notes/x.txt".to_string()]);
    let r = tool(&p, "a", "read_file", json!({"path": "notes/x.txt"})).await;
    assert_eq!(r.result.unwrap()["content"], "hello");
    assert_eq!(r.reads, vec!["file/notes/x.txt".to_string()]);
    let l = tool(&p, "a", "list_files", json!({})).await;
    assert_eq!(l.result, Some(json!([{"path": "notes/x.txt", "bytes": 5}])));
    assert_eq!(l.reads, vec!["file/*".to_string()]);
    let missing = tool(&p, "a", "read_file", json!({"path": "nope"})).await;
    assert!(missing.error.is_some());
}

async fn differing(p: &Folder, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

async fn inspect_keys(p: &Folder, target: Target, keys: &[&str]) -> Map<String, Value> {
    let keys = keys.iter().map(|k| k.to_string()).collect();
    p.inspect(InspectParams { target, keys })
        .await
        .unwrap()
        .docs
}

#[tokio::test]
async fn save_restore_inspect_diff() {
    let s = Saves::new("roundtrip");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    write(&p, "a", "one.txt", "1").await;
    let saved = save(&s, &p, "a", "s1").await;
    assert!(saved.exact && saved.bytes == 1);
    write(&p, "a", "one.txt", "changed").await;
    write(&p, "a", "two.txt", "2").await;
    let diff = differing(&p, target_i("a"), target_s("s1")).await;
    assert_eq!(diff, ["file/one.txt", "file/two.txt"]);
    p.start(start("b", json!({}), Some("s1"))).await.unwrap();
    assert!(differing(&p, target_i("b"), target_s("s1"))
        .await
        .is_empty());
    let docs = inspect_keys(&p, target_s("s1"), &["file/one.txt", "file/zzz", "other/x"]).await;
    assert_eq!(docs["file/one.txt"]["content"], "1");
    assert_eq!(
        (&docs["file/zzz"], &docs["other/x"]),
        (&Value::Null, &Value::Null)
    );
    assert_eq!(inspect_keys(&p, target_i("a"), &[]).await.len(), 2);
    // the live folder never writes through to a save
    write(&p, "b", "one.txt", "edited").await;
    assert_eq!(
        fs::read_to_string(s.0.join("folder/s1/one.txt")).unwrap(),
        "1"
    );
}

#[tokio::test]
async fn second_save_links_unchanged_files() {
    let s = Saves::new("links");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    write(&p, "a", "big.txt", "0123456789").await;
    assert_eq!(save(&s, &p, "a", "s1").await.bytes, 10);
    write(&p, "a", "new.txt", "xy").await;
    assert_eq!(save(&s, &p, "a", "s2").await.bytes, 2);
}

#[tokio::test]
async fn config_root_seeds_the_instance() {
    let s = Saves::new("seed");
    let seed = s.0.join("seed");
    fs::create_dir_all(seed.join("d")).unwrap();
    fs::write(seed.join("d/f"), "seeded").unwrap();
    let p = s.part();
    p.start(start("a", json!({"root": seed}), None))
        .await
        .unwrap();
    let r = tool(&p, "a", "read_file", json!({"path": "d/f"})).await;
    assert_eq!(r.result.unwrap()["content"], "seeded");
    let e = p
        .start(start("b", json!({"root": "/nonexistent/arena"}), None))
        .await;
    assert_eq!(e.unwrap_err().code, -32602);
}

#[tokio::test]
async fn unknown_things_and_stop() {
    let s = Saves::new("unknown");
    let p = s.part();
    let e = p
        .start(start("a", json!({}), Some("nope")))
        .await
        .unwrap_err();
    assert_eq!(e.code, -32002);
    let e = p
        .stop(IdParams {
            instance: "a".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32002);
    p.start(start("a", json!({}), None)).await.unwrap();
    assert!(s.0.join("folder/.live/a").is_dir());
    p.stop(IdParams {
        instance: "a".into(),
    })
    .await
    .unwrap();
    assert!(!s.0.join("folder/.live/a").exists());
    assert_eq!(p.usage().await.unwrap().bytes, 0);
}

#[tokio::test]
async fn drop_save_removes_its_folder() {
    let s = Saves::new("drop");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    write(&p, "a", "f", "1").await;
    save(&s, &p, "a", "s1").await;
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert!(!s.0.join("folder/s1").exists());
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
}

#[tokio::test]
async fn describe_is_exact_with_three_tools() {
    let s = Saves::new("describe");
    let d = s.part().describe().await.unwrap();
    assert_eq!(
        (d.restore.exact, d.restore.approximate),
        (ExactKind::Save, false)
    );
    let names: Vec<&str> = d.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["list_files", "read_file", "write_file"]);
}

fn build_all() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let a = arena();
        let run = |dir: PathBuf, args: &[&str]| {
            assert!(Command::new("cargo")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap()
                .success());
        };
        run(a.clone(), &["build", "-p", "arena-folder"]);
        run(
            a.join("core"),
            &["build", "-p", "parts", "--bin", "arena-part-check"],
        );
    });
}

#[test]
fn conformance_check_passes() {
    build_all();
    let a = arena();
    let saves = Saves::new("check");
    let out = Command::new(a.join("core/target/debug/arena-part-check"))
        .arg(a.join("integrations/folder/check"))
        .env("ARENA_BIN_DIR", a.join("core/target/debug"))
        .env("ARENA_SAVES", &saves.0)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success() && stdout.contains("\"passed\": true"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A folder with a file nobody can read, so copying it fails part way.
fn unreadable_folder(s: &Saves) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let d = s.0.join("seed");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("a"), "a").unwrap();
    fs::write(d.join("b"), "b").unwrap();
    fs::set_permissions(d.join("b"), fs::Permissions::from_mode(0o000)).unwrap();
    d
}

#[tokio::test]
async fn failed_start_leaves_no_folder() {
    let s = Saves::new("failstart");
    let p = s.part();
    let seed = unreadable_folder(&s);
    assert!(p
        .start(start("a", json!({"root": seed}), None))
        .await
        .is_err());
    assert!(!s.0.join("folder/.live/a").exists());
}

#[tokio::test]
async fn failed_save_leaves_no_folder() {
    use std::os::unix::fs::PermissionsExt;
    let s = Saves::new("failsave");
    let p = s.part();
    p.start(start("a", json!({}), None)).await.unwrap();
    write(&p, "a", "a", "a").await;
    write(&p, "a", "b", "b").await;
    let live = s.0.join("folder/.live/a/b");
    fs::set_permissions(&live, fs::Permissions::from_mode(0o000)).unwrap();
    let dir = s.save_dir("s1");
    let params = SaveParams {
        instance: "a".into(),
        save: "s1".into(),
        dir: dir.to_string_lossy().into(),
    };
    assert!(p.save(params).await.is_err());
    assert!(!dir.exists());
    assert!(p.drop_save(SaveId { save: "s1".into() }).await.is_ok());
}

/// A copy keeps folder modes: an instance started from (and saved with) a read-only folder still
/// stops, and its save drops.
#[tokio::test]
async fn stop_and_drop_save_remove_read_only_folders() {
    use std::os::unix::fs::PermissionsExt;
    let s = Saves::new("readonly");
    let seed = s.0.join("seed");
    fs::create_dir_all(seed.join("locked/inner")).unwrap();
    fs::write(seed.join("locked/inner/f"), "x").unwrap();
    for d in ["locked/inner", "locked"] {
        fs::set_permissions(seed.join(d), fs::Permissions::from_mode(0o500)).unwrap();
    }
    let p = s.part();
    p.start(start("a", json!({"root": seed}), None))
        .await
        .unwrap();
    save(&s, &p, "a", "s1").await;
    let mode = fs::metadata(s.0.join("folder/s1/locked"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o500);
    p.stop(IdParams {
        instance: "a".into(),
    })
    .await
    .unwrap();
    assert!(!s.0.join("folder/.live/a").exists());
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert!(!s.0.join("folder/s1").exists());
}
