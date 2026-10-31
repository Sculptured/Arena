//! Runs real Docker Compose projects: `cargo test -p arena-service -- --ignored live_`.
//! Every project, volume and folder the test makes is removed afterwards, even on failure.
use arena_part::proto::*;
use arena_part::Part;
use arena_service::Service;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const COMPOSE: &str = "services:\n  redis:\n    image: redis:6.2-alpine\n    entrypoint: [redis-server]\n    volumes:\n      - data:/data\nvolumes:\n  data: {}\n";

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

    /// Runs `args` in the instance's container and returns its trimmed output.
    fn exec(&self, instance: &str, args: &[&str]) -> String {
        let out = Command::new("docker")
            .args(["exec", &format!("arena_{instance}-redis-1")])
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

    fn redis(&self, instance: &str, args: &[&str]) -> String {
        let cli = [&["redis-cli"], args].concat();
        self.exec(instance, &cli)
    }

    /// Starts instance `id` (tracked for removal) from `from`.
    async fn start(&mut self, p: &Service, id: &str, from: Option<&str>) {
        self.projects.push(format!("arena_{id}"));
        p.start(start(id, self.config(), from)).await.unwrap();
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

async fn save_to(w: &World, p: &Service, instance: &str, id: &str) -> PathBuf {
    let dir = w.dir.join("saves/service").join(id);
    let params = SaveParams {
        instance: instance.into(),
        save: id.into(),
        dir: dir.to_string_lossy().into(),
    };
    let saved = p.save(params).await.unwrap();
    assert!(saved.exact && saved.bytes > 0, "{saved:?}");
    dir
}

/// Sets a key and a file with an unusual owner in instance `a`, then saves it as `s1`.
async fn first_save(w: &World, a: &str) -> PathBuf {
    assert_eq!(w.redis(a, &["set", "k", "before"]), "OK");
    w.exec(
        a,
        &["sh", "-c", "echo x > /data/own; chown 1234:4321 /data/own"],
    );
    // a program restarted since the instance began still saves it
    save_to(w, &w.part(), a, "s1").await
}

/// Changes `a` after the save: the change stays out of the save.
async fn diverge(w: &World, p: &Service, a: &str) {
    // the project runs again after the save
    assert_eq!(w.redis(a, &["get", "k"]), "before");
    assert_eq!(w.redis(a, &["set", "k", "after"]), "OK");
    assert_eq!(w.redis(a, &["save"]), "OK"); // a live diff sees what is on disk
    assert_eq!(differing(p, instance(a), save("s1")).await, ["volume/data"]);
}

/// The instance `b`, started from `s1`, has the saved key, the saved owner and the saved content.
async fn check_restored(w: &World, p: &Service, b: &str) {
    assert_eq!(w.redis(b, &["get", "k"]), "before");
    assert_eq!(
        w.exec(b, &["stat", "-c", "%u:%g", "/data/own"]),
        "1234:4321"
    );
    let same = differing(p, instance(b), save("s1")).await;
    assert!(same.is_empty(), "{same:?}");
    let keys = vec!["volume/data".into(), "volume/nope".into()];
    let docs = p
        .inspect(InspectParams {
            target: save("s1"),
            keys,
        })
        .await
        .unwrap()
        .docs;
    assert!(docs["volume/data"]["files"].as_u64().unwrap() >= 2);
    assert_eq!(docs["volume/nope"], Value::Null);
}

#[tokio::test]
#[ignore]
async fn live_service_roundtrip() {
    let id = std::process::id();
    let (a, b) = (format!("t10_{id}_a"), format!("t10_{id}_b"));
    let mut w = World::new();
    let p = w.part();
    w.start(&p, &a, None).await;
    let dir = first_save(&w, &a).await;
    diverge(&w, &p, &a).await;
    w.start(&p, &b, Some("s1")).await;
    check_restored(&w, &p, &b).await;
    assert_eq!(w.redis(&a, &["get", "k"]), "after");
    for i in [&a, &b] {
        w.part()
            .stop(IdParams {
                instance: i.clone(),
            })
            .await
            .unwrap();
    }
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
    assert!(!dir.exists());
}
