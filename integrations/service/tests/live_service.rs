//! Runs real Docker Compose projects: `cargo test -p arena-service -- --ignored live_`.
//! Every project, volume and folder the test makes is removed afterwards, even on failure.
use arena_part::proto::*;
use arena_part::Part;
use arena_service::Service;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const COMPOSE: &str = "services:\n  redis:\n    image: redis:6.2-alpine\n    volumes:\n      - data:/data\nvolumes:\n  data: {}\n";

/// The test's folder and compose projects; dropping it removes them all.
struct World {
    dir: PathBuf,
    projects: Vec<String>,
}

impl World {
    fn new() -> World {
        let dir = std::env::temp_dir().join(format!("arena_live_service_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("compose.yml"), COMPOSE).unwrap();
        World {
            dir,
            projects: vec![],
        }
    }

    fn part(&self) -> Service {
        Service::new(self.dir.join("saves/service"))
    }

    fn config(&self) -> Value {
        json!({"compose_file": self.dir.join("compose.yml"), "volumes": ["data"]})
    }

    /// The instance's project is removed on drop; named before it exists, so a failure is covered.
    fn track(&mut self, instance: &str) {
        self.projects.push(format!("arena_{instance}"));
    }

    fn redis(&self, instance: &str, args: &[&str]) -> String {
        let out = Command::new("docker")
            .args(["exec", &format!("arena_{instance}-redis-1"), "redis-cli"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        for p in &self.projects {
            let _ = Command::new("docker")
                .args(["compose", "-p", p, "down", "-v"])
                .env_remove("COMPOSE_PROJECT_NAME")
                .output();
        }
        let _ = fs::remove_dir_all(&self.dir);
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

fn instance(i: &str) -> Target {
    Target::Instance { instance: i.into() }
}

fn save(s: &str) -> Target {
    Target::Save { save: s.into() }
}

async fn differing(p: &Service, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

#[tokio::test]
#[ignore]
async fn live_service_roundtrip() {
    let id = std::process::id();
    let (a, b) = (format!("t10_{id}_a"), format!("t10_{id}_b"));
    let mut w = World::new();
    w.track(&a);
    w.track(&b);
    let p = w.part();
    p.start(start(&a, w.config(), None)).await.unwrap();
    assert_eq!(w.redis(&a, &["set", "k", "before"]), "OK");
    let dir = w.dir.join("saves/service/s1");
    let saved = p
        .save(SaveParams {
            instance: a.clone(),
            save: "s1".into(),
            dir: dir.to_string_lossy().into(),
        })
        .await
        .unwrap();
    assert!(saved.exact && saved.bytes > 0, "{saved:?}");
    // the project runs again after the save, and changes now stay out of the save
    assert_eq!(w.redis(&a, &["get", "k"]), "before");
    assert_eq!(w.redis(&a, &["set", "k", "after"]), "OK");
    assert_eq!(w.redis(&a, &["save"]), "OK"); // a live diff sees what is on disk
    assert_eq!(
        differing(&p, instance(&a), save("s1")).await,
        ["volume/data"]
    );
    p.start(start(&b, w.config(), Some("s1"))).await.unwrap();
    assert_eq!(w.redis(&b, &["get", "k"]), "before");
    assert_eq!(w.redis(&a, &["get", "k"]), "after");
    assert!(differing(&p, instance(&b), save("s1")).await.is_empty());
    let docs = p
        .inspect(InspectParams {
            target: save("s1"),
            keys: vec!["volume/data".into(), "volume/nope".into()],
        })
        .await
        .unwrap()
        .docs;
    assert!(docs["volume/data"]["files"].as_u64().unwrap() >= 1);
    assert!(docs["volume/data"]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(docs["volume/nope"], Value::Null);
    for i in [&a, &b] {
        p.stop(IdParams {
            instance: i.clone(),
        })
        .await
        .unwrap();
    }
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert!(!dir.exists());
}
