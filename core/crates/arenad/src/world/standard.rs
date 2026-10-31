//! The saves every run shares: the benchmark's base save (empty instances plus its `seed`) and
//! one gold save per task (the base plus the task's gold actions). Each is made once, under a
//! per-name lock, and an existing save counts as made.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use anyhow::{bail, Context, Result};
use parts::{
    proto::{Action, BenchDescribe, GoldActions},
    Hub,
};
use serde_json::{json, Value};

use crate::saves::save_exists;

use super::{
    call, exact,
    ids::{base_names, gold_names, temp_branch_id},
    plan::plan,
    Saves, StartSaves, World,
};

/// One lock per name for this process: a save is made once even when two runs start together.
async fn name_lock(name: &str) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(Default::default);
    let l = LOCKS
        .lock()
        .unwrap()
        .entry(name.to_string())
        .or_default()
        .clone();
    l.lock_owned().await
}

/// True when every save in `names` exists.
async fn saves_exist(hub: &Hub, bench: &BenchDescribe, names: &Saves) -> Result<bool> {
    for p in &bench.world {
        if !save_exists(hub, &p.integration, &names[&p.part]).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Makes `names` unless they exist: a short-lived world (empty when `from` is None), `build` on
/// it, then every part saved under its name.
async fn ensure_saves(
    hub: &Arc<Hub>,
    bench: &BenchDescribe,
    names: Saves,
    from: Option<StartSaves>,
    build: impl AsyncFnOnce(&World) -> Result<()>,
) -> Result<Saves> {
    let _g = name_lock(&names.values().cloned().collect::<Vec<_>>().join(",")).await;
    if saves_exist(hub, bench, &names).await? {
        return Ok(names);
    }
    let planned = plan(hub, bench).await?;
    let world = World::start_parts(
        hub.clone(),
        bench,
        planned,
        &temp_branch_id(),
        0,
        from.as_ref(),
    )
    .await?;
    let built = async {
        build(&world).await?;
        world.save_all_as(&names).await.map_err(anyhow::Error::from)
    }
    .await;
    world.stop().await;
    built.map(|()| names)
}

/// The base save of each part, made once (empty instances, the benchmark's `seed`, save).
pub async fn ensure_base(hub: &Arc<Hub>, bench: &BenchDescribe) -> Result<Saves> {
    let seed = async |w: &World| -> Result<()> {
        let _: Value = call(
            w.bench_client()?,
            &w.bench_label(),
            "seed",
            json!({"connections": w.connections}),
        )
        .await?;
        Ok(())
    };
    ensure_saves(hub, bench, base_names(bench), None, seed)
        .await
        .with_context(|| format!("making the base save of {}", bench.name))
}

/// The gold save of each part for `task`: the base plus the task's gold actions, made once.
pub async fn ensure_gold(hub: &Arc<Hub>, bench: &BenchDescribe, task: &str) -> Result<Saves> {
    let names = gold_names(bench, task);
    let from = exact(ensure_base(hub, bench).await?);
    ensure_saves(hub, bench, names, Some(from), async |w: &World| {
        apply_gold(w, task).await
    })
    .await
    .with_context(|| format!("making the gold save of task {task}"))
}

/// Runs the task's gold actions. At least one gold write must succeed (a failing gold action
/// changes nothing, as upstream).
async fn apply_gold(world: &World, task: &str) -> Result<()> {
    let params = json!({"task": task});
    let gold: GoldActions = call(
        world.bench_client()?,
        &world.bench_label(),
        "gold_actions",
        params,
    )
    .await?;
    let mut wrote = false;
    for Action { name, args } in gold.actions {
        let c = world.call_tool(&name, args, 0).await?;
        wrote |= c.output.error.is_none() && !c.output.writes.is_empty();
    }
    if !wrote {
        bail!("task {task}: no gold write succeeded");
    }
    Ok(())
}
