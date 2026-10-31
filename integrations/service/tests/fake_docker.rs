//! Service behaviour when Docker misbehaves, with a fake `docker` program first on the PATH.
use arena_part::proto::*;
use arena_part::Part;
use arena_service::Service;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// The fake: logs its arguments, and fails where the environment says so.
const FAKE: &str = r#"#!/bin/sh
echo "$@" >> "$FAKE_LOG"
case "$*" in
  *" stop") [ -n "$FAKE_STOP_FAIL" ] && { echo no stop >&2; exit 1; } ;;
  *" up "*) [ -n "$FAKE_UP_FAIL" ] && { echo no up >&2; exit 1; } ;;
  *-cpf*) case "$*" in *_bad:/v*) echo no export >&2; exit 1 ;; *) printf tar ;; esac ;;
esac
exit 0
"#;

/// A temporary folder with the fake docker; removed on drop.
struct Fake(PathBuf);

impl Fake {
    fn new(tag: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("arena_fake_{}_{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        let docker = dir.join("bin/docker");
        fs::write(&docker, FAKE).unwrap();
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("compose.yml"), "services: {}\n").unwrap();
        Fake(dir)
    }

    fn part(&self) -> Service {
        Service::new(self.0.join("saves/service"))
    }

    /// A config whose compose runs go to the fake; `extra` adds environment variables.
    fn config(&self, volumes: &[&str], extra: Value) -> Value {
        let path = format!(
            "{}:{}",
            self.0.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut env = json!({"PATH": path, "FAKE_LOG": self.log_path()});
        env.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"compose_file": self.0.join("compose.yml"), "volumes": volumes, "env": env})
    }

    fn log_path(&self) -> String {
        self.0.join("log").to_string_lossy().into()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.0.join("log")).unwrap_or_default()
    }

    fn live_file(&self, id: &str) -> PathBuf {
        self.0.join(format!("saves/service/.live/{id}.json"))
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn start(id: &str, config: Value) -> StartParams {
    StartParams {
        instance: id.into(),
        config,
        seed: 1,
        clock_ms: 0,
        from: None,
    }
}

fn save_params(f: &Fake, id: &str) -> SaveParams {
    let dir = f.0.join("saves/service/s1");
    SaveParams {
        instance: id.into(),
        save: "s1".into(),
        dir: dir.to_string_lossy().into(),
    }
}

#[tokio::test]
async fn failed_stop_still_starts_and_leaves_no_save() {
    let f = Fake::new("stop");
    let p = f.part();
    p.start(start(
        "a",
        f.config(&["data"], json!({"FAKE_STOP_FAIL": "1"})),
    ))
    .await
    .unwrap();
    let e = p.save(save_params(&f, "a")).await.unwrap_err();
    assert!(e.message.contains("no stop"), "{e}");
    assert!(f.log().trim_end().ends_with(" start"), "{}", f.log());
    assert!(!f.0.join("saves/service/s1").exists());
}

#[tokio::test]
async fn failed_export_leaves_no_partial_save() {
    let f = Fake::new("export");
    let p = f.part();
    p.start(start("a", f.config(&["data", "bad"], json!({}))))
        .await
        .unwrap();
    let e = p.save(save_params(&f, "a")).await.unwrap_err();
    assert!(e.message.contains("no export"), "{e}");
    assert!(
        f.log().trim_end().ends_with(" start"),
        "the project is started again"
    );
    assert!(!f.0.join("saves/service/s1").exists());
}

#[tokio::test]
async fn good_save_writes_archives() {
    let f = Fake::new("good");
    let p = f.part();
    p.start(start("a", f.config(&["data"], json!({}))))
        .await
        .unwrap();
    let saved = p.save(save_params(&f, "a")).await.unwrap();
    assert_eq!(saved.bytes, 3);
    assert!(f.0.join("saves/service/s1/data.tar").is_file());
}

#[tokio::test]
async fn failed_up_forgets_the_instance() {
    let f = Fake::new("up");
    let p = f.part();
    let e = p
        .start(start(
            "a",
            f.config(&["data"], json!({"FAKE_UP_FAIL": "1"})),
        ))
        .await
        .unwrap_err();
    assert!(e.message.contains("no up"), "{e}");
    assert!(!f.live_file("a").exists());
    assert!(f.log().contains(" down -v"), "a failed start is cleaned up");
}

#[tokio::test]
async fn failed_record_brings_nothing_up() {
    let f = Fake::new("record");
    fs::create_dir_all(f.0.join("saves/service")).unwrap();
    fs::write(f.0.join("saves/service/.live"), "in the way").unwrap();
    let e = f
        .part()
        .start(start("a", f.config(&["data"], json!({}))))
        .await
        .unwrap_err();
    assert_eq!(e.code, -32603);
    assert!(f.log().is_empty(), "docker ran: {}", f.log());
}

#[tokio::test]
async fn bad_helper_image_is_invalid_params() {
    let f = Fake::new("image");
    for image in ["", "-v", "a b", "x;y"] {
        let mut config = f.config(&["data"], json!({}));
        config["helper_image"] = json!(image);
        let e = f.part().start(start("a", config)).await.unwrap_err();
        assert_eq!(e.code, -32602, "{image:?}");
    }
    assert!(f.log().is_empty());
}
