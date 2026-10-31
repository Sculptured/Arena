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
    build_counter();
    let reg = Registry::from_dir(pkg_dir).unwrap();
    let name = reg.integrations().next().unwrap().name.clone();
    let hub = Hub::new(reg, arena().join("core/target/debug"), temp(tag));
    check(&hub, &name, json!({})).await
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
    let r = run(&flavour_pkg("broken_restore"), "broken").await;
    assert!(!r.passed);
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
    let r = run(&flavour_pkg("nondeterministic"), "nondet").await;
    assert!(!r.passed);
    assert_eq!(r.steps.last().unwrap().name, "replay");
}
