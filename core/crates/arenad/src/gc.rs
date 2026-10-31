//! Disk hygiene: the size the arena's databases use, and the startup sweep of orphans.

use anyhow::Result;
use world::snapshot::Worlds;

use crate::store::Store;

/// Database name prefixes that count toward the disk limit.
pub const COUNTED_PREFIXES: [&str; 3] = ["branch_", "snap_", "gold_"];

/// Temporary databases the service itself creates and normally drops right away
/// (`branch_rows_*`, `branch_goldcopy_*` are `branch_` databases and so are covered by the
/// orphan rule). A crash can leave a half-built gold behind; nothing else is touched.
const TEMP_PREFIXES: [&str; 1] = ["gold_tmp_"];

/// (number of databases, total bytes) of the `branch_`, `snap_` and `gold_` databases.
pub async fn usage(worlds: &Worlds) -> Result<(usize, i64)> {
    worlds.admin().usage(&COUNTED_PREFIXES).await
}

/// Drops the `branch_*` and `snap_*` databases that no branch or event in the store refers
/// to (left by crashes or aborted tests), and leftover temporary databases. Never touches
/// `world_base`, `gold_*`, `arena` or databases with any other prefix. Returns
/// (databases dropped, bytes they used).
pub async fn collect_orphans(store: &Store, worlds: &Worlds) -> Result<(usize, i64)> {
    let keep = store.referenced_dbs().await?;
    let prefixes: Vec<&str> = ["branch_", "snap_"]
        .into_iter()
        .chain(TEMP_PREFIXES)
        .collect();
    collect_orphans_with(worlds, &prefixes, &keep).await
}

/// The selection behind `collect_orphans`: drops every database whose name starts with one
/// of `prefixes` (exact text, not a LIKE pattern) and is not in `keep`. Tests pass unique
/// prefixes so a real server's databases can't be selected.
pub async fn collect_orphans_with(
    worlds: &Worlds,
    prefixes: &[&str],
    keep: &std::collections::HashSet<String>,
) -> Result<(usize, i64)> {
    let mut candidates = vec![];
    for p in prefixes {
        candidates.extend(worlds.list(p).await?);
    }
    let (mut n, mut bytes) = (0, 0);
    for (name, size) in candidates {
        if keep.contains(&name) {
            continue;
        }
        match worlds.drop_db(&name).await {
            Ok(()) => {
                n += 1;
                bytes += size;
            }
            Err(e) => eprintln!("gc: could not drop {name}: {e:#}"),
        }
    }
    Ok((n, bytes))
}

/// `collect_orphans` plus the `gc: dropped N databases (X MB)` log line.
pub async fn run_startup_gc(store: &Store, worlds: &Worlds) -> Result<(usize, i64)> {
    let (n, bytes) = collect_orphans(store, worlds).await?;
    println!("gc: dropped {n} databases ({} MB)", bytes / 1_000_000);
    Ok((n, bytes))
}
