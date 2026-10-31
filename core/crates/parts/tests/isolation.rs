//! Package isolation: integrations, benchmarks and kits never depend on core crates, and core crates
//! never depend on integrations or benchmarks.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn arena_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

fn metadata(manifest: &Path) -> Value {
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

type Pkg = (String, PathBuf, Vec<(String, PathBuf)>);

/// (package name, manifest folder, path dependencies as (name, path)).
fn packages(manifest: &Path) -> Vec<Pkg> {
    let meta = metadata(manifest);
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let dir = Path::new(p["manifest_path"].as_str().unwrap())
                .parent()
                .unwrap()
                .to_path_buf();
            let deps = p["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|d| {
                    Some((
                        d["name"].as_str()?.to_string(),
                        PathBuf::from(d["path"].as_str()?),
                    ))
                })
                .collect();
            (p["name"].as_str().unwrap().to_string(), dir, deps)
        })
        .collect()
}

#[test]
fn packages_and_core_stay_apart() {
    let root = arena_root();
    let (core, integrations, benchmarks, kits) = (
        root.join("core"),
        root.join("integrations"),
        root.join("benchmarks"),
        root.join("kits"),
    );
    let mut all = packages(&core.join("Cargo.toml"));
    all.extend(packages(&root.join("Cargo.toml")));
    assert!(all.iter().any(|p| p.0 == "parts") && all.iter().any(|p| p.0 == "arena-part"));
    for (name, dir, deps) in &all {
        let outside = [&integrations, &benchmarks, &kits]
            .iter()
            .any(|r| dir.starts_with(r));
        for (dep, path) in deps {
            if outside {
                assert!(
                    !path.starts_with(&core),
                    "{name} depends on core crate {dep}"
                );
            }
            if dir.starts_with(&core) {
                assert!(
                    !path.starts_with(&integrations) && !path.starts_with(&benchmarks),
                    "core crate {name} depends on {dep}"
                );
            }
        }
    }
}
