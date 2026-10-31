//! Disk hygiene: the bytes the arena keeps, deleting a run's saves, and the startup cleanup of
//! saves no event names.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use parts::{
    proto::{kept, GcResult, WorldPart},
    Hub,
};

use crate::{
    model::Event,
    recorded::{named_saves, NamedSave},
    saves::{self, integration_of, integrations, world_of},
    store::Store,
    world::instance_id,
};

/// Bytes the arena keeps: the `usage` of each integration `benchmark`'s world uses, plus the
/// save folders.
pub async fn usage(hub: &Hub, benchmark: &str) -> Result<u64> {
    let mut bytes = hub.saves_bytes();
    for i in integrations(hub, benchmark).await? {
        bytes += saves::usage(hub, &i).await?;
    }
    Ok(bytes)
}

/// The shared saves of every benchmark and version, as `gc` keep prefixes.
const SHARED_KEEP: [&str; 2] = ["base_*", "gold_*"];

/// The shared saves every run starts from or is checked against: never dropped with a run.
fn is_shared(save: &str) -> bool {
    kept(&SHARED_KEEP, save)
}

/// Per integration, what deleting a run frees: the saves to drop and the instances to stop.
#[derive(Default)]
struct Doomed {
    saves: BTreeMap<String, BTreeSet<String>>,
    instances: BTreeMap<String, BTreeSet<String>>,
}

impl Doomed {
    /// Adds one branch: the saves its events name (but the shared ones) and its instances.
    fn add_branch(&mut self, branch: &str, events: &[Event], world: &[WorldPart]) {
        for NamedSave { part, save } in named_saves(branch, events) {
            if is_shared(&save) {
                continue;
            }
            match integration_of(world, &part) {
                Some(i) => {
                    self.saves.entry(i.to_string()).or_default().insert(save);
                }
                None => {
                    eprintln!("delete: no integration for part {part} of {branch}; {save} kept")
                }
            }
        }
        for p in world {
            let instance = instance_id(branch, &p.part);
            self.instances
                .entry(p.integration.clone())
                .or_default()
                .insert(instance);
        }
    }
}

/// Deletes the run from the store, then drops every save its events name (except the shared
/// base and gold saves) and stops its instances. The number of saves dropped.
pub async fn delete_run(store: &Store, hub: &Hub, run_id: &str) -> Result<usize> {
    let mut doomed = Doomed::default();
    for b in store.run_branches(run_id).await? {
        let events = store.events(&b).await?;
        let world = world_of(hub, &events).await?;
        doomed.add_branch(&b, &events, &world);
    }
    store.delete_run(run_id).await?;
    Ok(drop_doomed(hub, &doomed, run_id).await)
}

/// Drops the saves and stops the instances; failures are logged. The number of saves dropped
/// (a save that was already gone doesn't count).
async fn drop_doomed(hub: &Hub, doomed: &Doomed, run_id: &str) -> usize {
    let mut dropped = 0;
    for (integration, list) in &doomed.saves {
        for save in list {
            match saves::drop_save(hub, integration, save).await {
                Ok(existed) => dropped += usize::from(existed),
                Err(e) => eprintln!("delete run {run_id}: could not drop {save}: {e:#}"),
            }
        }
    }
    for (integration, list) in &doomed.instances {
        for instance in list {
            if let Err(e) = saves::stop(hub, integration, instance).await {
                eprintln!("delete run {run_id}: could not stop {instance}: {e:#}");
            }
        }
    }
    dropped
}

/// Every save the store's events name, plus the base and gold saves of every benchmark.
async fn keep_list(store: &Store) -> Result<Vec<String>> {
    let mut by_branch: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    for e in store.events_naming_saves().await? {
        by_branch.entry(e.branch_id.clone()).or_default().push(e);
    }
    let mut keep: BTreeSet<String> = by_branch
        .iter()
        .flat_map(|(b, events)| named_saves(b, events))
        .map(|n| n.save)
        .collect();
    keep.extend(SHARED_KEEP.map(String::from));
    Ok(keep.into_iter().collect())
}

/// Asks each integration `benchmark`'s world uses to drop the saves no event names (and its
/// instances, none of which runs yet), and removes the save folders not kept. Integrations
/// without `gc` are skipped; a failing one is logged. Logs `gc: dropped N saves (X MB)`.
pub async fn run_startup_gc(store: &Store, hub: &Hub, benchmark: &str) -> Result<GcResult> {
    let keep = keep_list(store).await?;
    let mut total = GcResult::default();
    for i in integrations(hub, benchmark).await? {
        match saves::gc(hub, &i, &keep).await {
            Ok(Some(r)) => {
                total.dropped += r.dropped;
                total.bytes += r.bytes;
            }
            Ok(None) => println!("gc: the {i} integration has no gc; skipped"),
            Err(e) => eprintln!("gc: {e:#}"),
        }
    }
    println!(
        "gc: dropped {} saves ({} MB)",
        total.dropped,
        total.bytes / 1_000_000
    );
    Ok(total)
}
