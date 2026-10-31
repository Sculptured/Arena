use parts::{check, Hub, Registry, Report};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
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
    let d = std::env::temp_dir().join(format!("check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

async fn run(pkg_dir: &Path, tag: &str) -> Report {
    run_in(pkg_dir, &temp(tag)).await.0
}

/// The report, and the saves root with the bytes `usage` of the part gave afterwards.
async fn run_in(pkg_dir: &Path, saves: &Path) -> (Report, u64) {
    build_counter();
    let reg = Registry::from_dir(pkg_dir).unwrap();
    let name = reg.integrations().next().unwrap().name.clone();
    let hub = Hub::new(reg, arena().join("core/target/debug"), saves.to_path_buf());
    let r = check(&hub, &name, json!({})).await;
    let c = hub.client(&name).await.unwrap();
    let u: serde_json::Value = c.call("usage", json!({})).await.unwrap();
    (r, u["bytes"].as_u64().unwrap())
}

/// Nothing of the package is left under the saves root.
fn assert_nothing_left(saves: &Path, usage: u64) {
    let left: Vec<_> = std::fs::read_dir(saves.join("counter"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "saves left behind: {left:?}");
    assert!(usage <= 4096, "usage is {usage}");
}

const FAKE: &str = r#"
import json, sys
mode = sys.argv[1]
saves = set()
def tools():
    return [{"name": "add", "description": "", "inputSchema": {"type": "object"}, "effect": "write"},
            {"name": "get", "description": "", "inputSchema": {"type": "object"}, "effect": "read"}]
for line in sys.stdin:
    r = json.loads(line); m = r["method"]; p = r["params"]; err = None; res = {}
    if m == "describe":
        res = {"name": "fake", "version": "0", "protocol": 1,
               "restore": {"exact": "replay" if mode == "empty_inspect" else "save", "approximate": False},
               "uses_clock": False, "fingerprint": {}, "tools": tools()}
    elif m == "call_tool":
        res = {"result": {}, "reads": [], "writes": ["fake/x"] if mode != "no_keys" else []}
    elif m == "save":
        if mode == "empty_inspect":
            err = {"code": -32001, "message": "no saves"}
        else:
            saves.add(p["save"]); res = {"bytes": 1, "exact": True}
    elif m == "diff":
        res = {} if mode == "bad_diff" else {"differing": []}
    elif m == "inspect":
        res = {"docs": {}}
    elif m == "drop_save":
        if mode == "drop_fails" and p["save"] == "chk_s1":
            err = {"code": -32002, "message": "cannot drop"}
    elif m == "usage":
        res = {"bytes": 0}
    out = {"jsonrpc": "2.0", "id": r["id"]}
    out.update({"error": err} if err else {"result": res})
    print(json.dumps(out), flush=True)
"#;

fn fake_pkg(mode: &str, examples: bool) -> PathBuf {
    let d = temp(&format!("fake-{mode}-{examples}"));
    std::fs::write(d.join("fake.py"), FAKE).unwrap();
    let ex = if examples {
        "[[example]]\ntool = \"add\"\nargs = {}\n"
    } else {
        ""
    };
    std::fs::write(
        d.join("arena-package.toml"),
        format!(
            "kind = \"integration\"\nname = \"counter\"\nversion = \"0\"\nprotocol = 1\n\
             run = {{ command = [\"python3\", \"fake.py\", \"{mode}\"] }}\n{ex}"
        ),
    )
    .unwrap();
    d
}

/// A counter package in a temp dir, started with `--flavour <flavour>`.
fn flavour_pkg(flavour: &str) -> PathBuf {
    let d = temp(&format!("pkg-{flavour}"));
    std::fs::write(
        d.join("arena-package.toml"),
        format!(
            "kind = \"integration\"\nname = \"counter\"\nversion = \"0.1.0\"\nprotocol = 1\n\
             run = {{ command = [\"${{ARENA_BIN_DIR}}/examples/counter\", \"--flavour\", \"{flavour}\"] }}\n\n\
             [[example]]\ntool = \"add\"\nargs = {{ n = 1 }}\n\n[[example]]\ntool = \"get\"\nargs = {{}}\n"
        ),
    )
    .unwrap();
    d
}

fn names(r: &Report) -> Vec<&str> {
    r.steps.iter().map(|s| s.name.as_str()).collect()
}

#[tokio::test]
async fn counter_passes() {
    let r = run(&arena().join("kits/rust/examples/counter"), "rust").await;
    assert!(r.passed, "{}", serde_json::to_string_pretty(&r).unwrap());
    assert_eq!(r.steps.len(), 10); // all but replay
    assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped); // replay does not apply
}

#[tokio::test]
async fn python_counter_passes() {
    let r = run(&arena().join("kits/python/examples"), "py").await;
    assert!(r.passed, "{}", serde_json::to_string_pretty(&r).unwrap());
    // The Python counter has no write example, so the write and second save are skipped.
    assert!(r.skipped.iter().any(|s| s.starts_with("write_example")));
}

#[tokio::test]
async fn broken_restore_fails() {
    let saves = temp("broken-saves");
    let (r, usage) = run_in(&flavour_pkg("broken_restore"), &saves).await;
    assert!(!r.passed);
    assert_nothing_left(&saves, usage);
    assert!(r
        .skipped
        .iter()
        .any(|s| s.starts_with("not run: diff_self")));
    let last = r.steps.last().unwrap();
    assert_eq!(last.name, "restore");
    assert!(!last.ok);
    assert_eq!(r.steps.len(), 7);
}

#[tokio::test]
async fn replay_only_passes_by_inspect() {
    let r = run(&flavour_pkg("replay_only"), "replay").await;
    assert!(r.passed, "{}", serde_json::to_string_pretty(&r).unwrap());
    assert!(r.skipped.iter().any(|s| s.starts_with("chk_s1")));
    assert!(names(&r).contains(&"replay"));
}

#[tokio::test]
async fn approx_and_replay_passes() {
    let r = run(&flavour_pkg("approx_and_replay"), "approx").await;
    assert!(r.passed, "{}", serde_json::to_string_pretty(&r).unwrap());
}

#[tokio::test]
async fn nondeterministic_fails_at_replay() {
    let saves = temp("nondet-saves");
    let (r, usage) = run_in(&flavour_pkg("nondeterministic"), &saves).await;
    assert!(!r.passed);
    assert_nothing_left(&saves, usage);
    assert_eq!(r.steps.last().unwrap().name, "replay");
}

fn failed(r: &Report) -> &parts::Step {
    assert!(!r.passed);
    r.steps.iter().find(|s| !s.ok).expect("a failed step")
}

#[tokio::test]
async fn malformed_diff_fails() {
    let r = run(&fake_pkg("bad_diff", true), "bad-diff").await;
    let f = failed(&r);
    assert_eq!(f.name, "restore", "{r:?}");
    assert!(f.detail.contains("differing"), "{}", f.detail);
}

#[tokio::test]
async fn replay_without_keys_or_docs_fails() {
    let r = run(&fake_pkg("empty_inspect", true), "empty-docs").await;
    let f = failed(&r);
    assert_eq!(f.name, "replay", "{r:?}");
    assert!(f.detail.contains("no docs"), "{}", f.detail);
    let r = run(&fake_pkg("no_keys", true), "no-keys").await;
    assert!(r.passed, "exact part: replay is not required");
}

#[tokio::test]
async fn no_examples_fails() {
    let r = run(&fake_pkg("ok", false), "no-examples").await;
    let f = failed(&r);
    assert_eq!(f.name, "examples");
    assert!(f.detail.contains("[[example]]"));
}

#[tokio::test]
async fn failed_drop_is_a_failure_and_the_rest_are_tried() {
    let r = run(&fake_pkg("drop_fails", true), "drop-fails").await;
    let f = failed(&r);
    assert_eq!(f.name, "drop_saves", "{r:?}");
    // chk_s1 can't be dropped, so cleanup reports it too, and chk_s2 is not leaked silently.
    let c = r.steps.iter().find(|s| s.name == "cleanup").unwrap();
    assert!(!c.ok && c.detail.contains("chk_s1"), "{}", c.detail);
    assert!(!c.detail.contains("chk_s2"));
}
