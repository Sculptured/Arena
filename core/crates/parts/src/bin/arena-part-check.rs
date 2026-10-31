//! `arena-part-check <package dir> [--config file.json]`: the conformance check of one package.
use parts::{check, Hub, Registry};
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() {
    std::process::exit(match run().await {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("arena-part-check: {e:#}");
            2
        }
    });
}

async fn run() -> anyhow::Result<bool> {
    let mut dir = None;
    let mut config = serde_json::json!({});
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" {
            let f = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("--config needs a file"))?;
            config = serde_json::from_str(&std::fs::read_to_string(&f)?)?;
        } else if dir.is_none() {
            dir = Some(PathBuf::from(a));
        } else {
            anyhow::bail!("usage: arena-part-check <package dir> [--config file.json]");
        }
    }
    let dir = dir.ok_or_else(|| {
        anyhow::anyhow!("usage: arena-part-check <package dir> [--config file.json]")
    })?;
    let arena = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let bin_dir = std::env::var_os("ARENA_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| arena.join("core/target/release"));
    let (saves, temp) = match std::env::var_os("ARENA_SAVES") {
        Some(s) => (PathBuf::from(s), false),
        None => (unique_temp_dir()?, true),
    };
    let registry = Registry::from_dir(&dir)?;
    let name = registry
        .integrations()
        .chain(registry.benchmarks())
        .next()
        .unwrap()
        .name
        .clone();
    let hub = Hub::new(registry, bin_dir, saves.clone());
    let report = check(&hub, &name, config).await;
    println!("{}", serde_json::to_string_pretty(&report)?);
    drop(hub);
    if temp {
        let _ = std::fs::remove_dir_all(&saves);
    }
    Ok(report.passed)
}

/// A fresh folder under the temp dir; never one that already existed.
fn unique_temp_dir() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for n in 0..100 {
        let d = std::env::temp_dir().join(format!(
            "arena-part-check-{}-{nanos}-{n}",
            std::process::id()
        ));
        match std::fs::create_dir(&d) {
            Ok(()) => return Ok(d),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temp folder"))
}
