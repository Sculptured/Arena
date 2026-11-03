//! Image mode, with a fake `docker` on PATH (this test binary changes PATH, so it holds one test).
use parts::{Client, Manifest};
use std::path::{Path, PathBuf};

/// Lays out a scratch folder with a fake `docker` that logs its arguments, an image-mode package
/// and a saves folder, and puts the fake on PATH. Returns the folder, package, saves and log.
fn setup_fake_docker() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let d = std::env::temp_dir().join(format!("parts-image-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let (bin, pkg, saves) = (d.join("bin"), d.join("pkg"), d.join("saves"));
    for p in [&bin, &pkg, &saves] {
        std::fs::create_dir_all(p).unwrap();
    }
    let log = d.join("docker.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {}\ncase \"$1\" in run) sleep 30;; esac\n",
        log.display()
    );
    let fake = bin.join("docker");
    std::fs::write(&fake, script).unwrap();
    let mut perm = std::fs::metadata(&fake).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&fake, perm).unwrap();
    std::fs::write(
        pkg.join("arena-package.toml"),
        "kind = \"integration\"\nname = \"boxed\"\nversion = \"0.1.0\"\nprotocol = 1\nrun = { image = \"img:1\" }\n",
    )
    .unwrap();
    std::env::set_var(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    (d, pkg, saves, log)
}

/// Polls the fake docker's log until it contains `needle`, at most `tries` times.
async fn wait_for_log(log: &Path, needle: &str, tries: u32, every_ms: u64) -> String {
    let mut text = String::new();
    for _ in 0..tries {
        text = std::fs::read_to_string(log).unwrap_or_default();
        if text.contains(needle) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(every_ms)).await;
    }
    text
}

#[tokio::test]
async fn container_is_named_and_removed_on_drop() {
    let (d, pkg, saves, log) = setup_fake_docker();
    let m = Manifest::load(&pkg).unwrap();
    let c = Client::spawn(&m, &PathBuf::from("/x"), &saves)
        .await
        .unwrap();
    wait_for_log(&log, "run", 100, 20).await;
    drop(c);
    let text = wait_for_log(&log, "rm -f", 40, 50).await;
    let name = format!("arena-part-boxed-{}-", std::process::id());
    assert!(text.contains(&format!("--name {name}")), "{text}");
    assert!(text.contains(&format!("rm -f {name}")), "{text}");
    assert!(text.contains("--network none"), "{text}");
    std::fs::remove_dir_all(&d).unwrap();
}
