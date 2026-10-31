//! Service tests that need no Docker: config checks, ids and the part's description.
use arena_part::proto::*;
use arena_part::Part;
use arena_service::Service;
use serde_json::{json, Value};
use std::path::PathBuf;

fn part() -> Service {
    Service::new(
        std::env::temp_dir().join(format!("arena_service_nodocker_{}", std::process::id())),
    )
}

fn start(instance: &str, config: Value) -> StartParams {
    StartParams {
        instance: instance.into(),
        config,
        seed: 1,
        clock_ms: 0,
        from: None,
    }
}

#[tokio::test]
async fn describe_is_exact_and_toolless() {
    let d = part().describe().await.unwrap();
    assert_eq!(
        (d.restore.exact, d.restore.approximate),
        (ExactKind::Save, false)
    );
    assert!(d.tools.is_empty());
    assert_eq!(d.name, "service");
}

#[tokio::test]
async fn bad_config_is_invalid_params() {
    let p = part();
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let bad = [
        json!({}),
        json!({"compose_file": here, "volumes": ["../x"]}),
        json!({"compose_file": here, "volumes": ["a b"]}),
        json!({"compose_file": "/nonexistent/compose.yml", "volumes": ["data"]}),
        json!({"compose_file": here, "volumes": "data"}),
    ];
    for config in bad {
        let e = p.start(start("a", config.clone())).await.unwrap_err();
        assert_eq!(e.code, -32602, "{config}");
    }
}

#[tokio::test]
async fn unknown_instance_and_save() {
    let p = part();
    let e = p
        .stop(IdParams {
            instance: "a".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32002);
    let e = p
        .save(SaveParams {
            instance: "a".into(),
            save: "s".into(),
            dir: "/nonexistent".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32002);
    let e = p
        .inspect(InspectParams {
            target: Target::Save { save: "s".into() },
            keys: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32002);
    assert_eq!(p.usage().await.unwrap().bytes, 0);
}
