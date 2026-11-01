//! A start from a save is refused (-32001) before anything runs when the save was made by
//! another sculptured build, or holds more waiting changes than can be proposed again.
//! `/bin/echo` stands in for the program: its `--version` is the build.
use arena_part::proto::{FromSave, Mode, StartParams};
use arena_part::Part;
use arena_sculptured::Sculptured;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const ECHO: &str = "/bin/echo";

fn echo_build() -> String {
    let out = Command::new(ECHO)
        .arg("--version")
        .env_clear()
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A save `id` made by `build`, holding `waiting` changes still waiting for a decision.
fn fake_save(saves: &Path, id: &str, build: &str, waiting: usize) {
    let dir = saves.join(id);
    fs::create_dir_all(dir.join("data")).unwrap();
    let change = |n: usize| {
        json!({"id": format!("chg-{n}"), "action": "a", "summary": "s",
               "payload": {}, "decision": null})
    };
    let changes: Vec<Value> = (0..waiting).map(change).collect();
    fs::write(dir.join("changes.json"), json!(changes).to_string()).unwrap();
    let meta = json!({"build": build, "aliases": []});
    fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
}

async fn start_from(part: &Sculptured, save: &str) -> arena_part::PartError {
    let p = StartParams {
        instance: "r1".into(),
        config: json!({"binary": ECHO}),
        seed: 1,
        clock_ms: 0,
        from: Some(FromSave {
            save: save.into(),
            mode: Mode::Exact,
        }),
    };
    part.start(p).await.unwrap_err()
}

#[tokio::test]
async fn restores_refused_before_anything_runs() {
    let t = Tmp(std::env::temp_dir().join(format!("arena_sc_rc_{}", std::process::id())));
    let saves = t.0.join("saves/sculptured");
    fake_save(&saves, "other_build", "sculptured 0.0.1", 1);
    fake_save(&saves, "too_many", &echo_build(), 9);
    let part = Sculptured::new(saves.clone());
    let e = start_from(&part, "other_build").await;
    assert_eq!(e.code, -32001, "{e}");
    assert!(e.message.contains("was made by sculptured 0.0.1"), "{e}");
    let e = start_from(&part, "too_many").await;
    assert_eq!(e.code, -32001, "{e}");
    assert!(e.message.contains("9 changes waiting"), "{e}");
    assert!(!saves.join(".live/r1").exists(), "nothing was started");
}
