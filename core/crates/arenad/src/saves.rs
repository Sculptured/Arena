//! Calls on saves outside a running world, by integration: read keys, compare, drop, stop a
//! leftover instance, measure and clean up. A part's own error stays a `parts::CallError` inside
//! the returned error, so callers can downcast it for its code.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use parts::{
    proto::{Differing, Docs, GcResult, Usage, WorldPart},
    CallError, Hub,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::{model::Event, recorded::run_info};

async fn call<T: DeserializeOwned>(
    hub: &Hub,
    integration: &str,
    method: &str,
    params: Value,
) -> Result<T> {
    let client = hub.client(integration).await?;
    client
        .call(method, params)
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("the {integration} integration: {method}"))
}

/// The part's own error code, when `e` is one.
pub fn part_code(e: &anyhow::Error) -> Option<i64> {
    match e.downcast_ref::<CallError>() {
        Some(CallError::Part { code, .. }) => Some(*code),
        _ => None,
    }
}

/// The docs of `keys` in `save` (empty for no keys: `inspect` with none would read everything).
pub async fn inspect(
    hub: &Hub,
    integration: &str,
    save: &str,
    keys: &[String],
) -> Result<Map<String, Value>> {
    if keys.is_empty() {
        return Ok(Map::new());
    }
    let params = json!({"target": {"save": save}, "keys": keys});
    let d: Docs = call(hub, integration, "inspect", params).await?;
    Ok(d.docs)
}

/// The keys that differ between two saves of one integration.
pub async fn diff(hub: &Hub, integration: &str, a: &str, b: &str) -> Result<Vec<String>> {
    let params = json!({"a": {"save": a}, "b": {"save": b}});
    let d: Differing = call(hub, integration, "diff", params).await?;
    Ok(d.differing)
}

/// Drops `save` and the folder arenad made for it.
pub async fn drop_save(hub: &Hub, integration: &str, save: &str) -> Result<()> {
    let _: Value = call(hub, integration, "drop_save", json!({"save": save})).await?;
    hub.remove_save_dir(integration, save);
    Ok(())
}

/// Stops `instance` (a no-op for an instance that is not running).
pub async fn stop(hub: &Hub, integration: &str, instance: &str) -> Result<()> {
    let _: Value = call(hub, integration, "stop", json!({"instance": instance})).await?;
    Ok(())
}

/// Bytes the integration keeps.
pub async fn usage(hub: &Hub, integration: &str) -> Result<u64> {
    let u: Usage = call(hub, integration, "usage", json!({})).await?;
    Ok(u.bytes)
}

/// Drops the integration's saves not in `keep` (and its instances not running); None when the
/// integration has no `gc`.
pub async fn gc(hub: &Hub, integration: &str, keep: &[String]) -> Result<Option<GcResult>> {
    match call(hub, integration, "gc", json!({"keep": keep})).await {
        Ok(r) => Ok(Some(r)),
        Err(e) if part_code(&e) == Some(-32601) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The parts of a branch's world with their integrations: as its `RunStart` recorded them, else
/// (runs recorded before integrations) as the run's benchmark names them now.
pub async fn world_of(hub: &Hub, events: &[Event]) -> Result<Vec<WorldPart>> {
    let info = run_info(events);
    if !info.world.is_empty() {
        return Ok(info.world);
    }
    Ok(hub.bench_describe(&info.benchmark).await?.world)
}

/// The integration of `part` in `world`.
pub fn integration_of<'a>(world: &'a [WorldPart], part: &str) -> Option<&'a str> {
    world
        .iter()
        .find(|p| p.part == part)
        .map(|p| p.integration.as_str())
}

/// The integrations `benchmark`'s world uses, each once.
pub async fn integrations(hub: &Hub, benchmark: &str) -> Result<BTreeSet<String>> {
    let bench = hub.bench_describe(benchmark).await?;
    Ok(bench.world.into_iter().map(|p| p.integration).collect())
}
