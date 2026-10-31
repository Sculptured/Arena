//! A branch's world: one instance of each part of the benchmark's world, run by the part's
//! integration program (shared by every branch through the hub), plus the benchmark program for
//! the benchmark's own tools.

mod ids;
mod plan;
mod standard;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, bail, Context, Result};
use parts::{
    proto::{
        BenchDescribe, BenchToolOutput, Describe, Differing, Effect, ExactKind, Mode, ToolOutput,
        ToolSchema, WorldPart,
    },
    CallError, Client, Hub,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

pub use ids::{instance_id, new_save_id};
pub use plan::{plan, Plan, PlannedPart, Route};
pub use standard::{ensure_base, ensure_gold};

/// `{part: save}`.
pub type Saves = BTreeMap<String, String>;

/// Per part: the save it starts from and how to restore it.
pub type StartSaves = BTreeMap<String, (String, Mode)>;

/// Each part restored exactly from its save.
pub fn exact(saves: Saves) -> StartSaves {
    saves
        .into_iter()
        .map(|(p, s)| (p, (s, Mode::Exact)))
        .collect()
}

/// Where a world starts from.
pub enum WorldFrom {
    /// The benchmark's base save (made on first use).
    Base,
    /// Per part: the save and how to restore it.
    Saves(StartSaves),
}

impl WorldFrom {
    /// Each part restored exactly from its save.
    pub fn exact(saves: Saves) -> WorldFrom {
        WorldFrom::Saves(exact(saves))
    }
}

/// A failed world call. `fatal` when a program has stopped: the branch can't go on.
#[derive(Debug, Clone)]
pub struct WorldError {
    pub message: String,
    pub fatal: bool,
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for WorldError {}

impl WorldError {
    fn other(message: String) -> WorldError {
        WorldError {
            message,
            fatal: false,
        }
    }
}

/// What a tool call gave back, and which part it ran on (None for an unknown tool).
#[derive(Debug, Clone)]
pub struct Called {
    pub output: ToolOutput,
    pub part: Option<String>,
}

struct Part {
    name: String,
    integration: String,
    instance: String,
    client: Arc<Client>,
    describe: Describe,
}

impl Part {
    fn label(&self) -> String {
        format!("the {} integration", self.name)
    }
}

#[derive(Default)]
struct Track {
    /// The save that holds each part's current state.
    last: Saves,
    /// Parts written since their last save (a save failed).
    dirty: BTreeSet<String>,
    stopped: bool,
}

pub struct World {
    hub: Arc<Hub>,
    bench: String,
    bench_client: Option<Arc<Client>>,
    parts: Vec<Part>,
    tools: Vec<(ToolSchema, Route)>,
    connections: Map<String, Value>,
    track: Mutex<Track>,
    started: Instant,
}

fn stopped_message(what: &str, tail: &str) -> String {
    match tail.trim() {
        "" => format!("{what} stopped"),
        tail => format!("{what} stopped: {tail}"),
    }
}

/// A call to a program; a stopped program is a fatal `WorldError` naming `what`.
async fn call<T: DeserializeOwned>(
    client: &Client,
    what: &str,
    method: &str,
    params: Value,
) -> Result<T, WorldError> {
    client.call(method, params).await.map_err(|e| match e {
        CallError::Stopped(tail) => WorldError {
            message: stopped_message(what, &tail),
            fatal: true,
        },
        other => WorldError::other(format!("{what}: {method} failed: {other}")),
    })
}

impl World {
    /// Starts `bench`'s world for `branch_id`: one instance per part, from the base save or from
    /// the given saves, all with `seed` at clock 0. Refuses a world with a tool-name clash.
    pub async fn start(
        hub: Arc<Hub>,
        bench: &BenchDescribe,
        branch_id: &str,
        seed: u64,
        from: WorldFrom,
    ) -> Result<World> {
        let planned = plan(&hub, bench).await?;
        World::start_planned(hub, bench, planned, branch_id, seed, from).await
    }

    /// `start` with the world's `plan` already made.
    pub async fn start_planned(
        hub: Arc<Hub>,
        bench: &BenchDescribe,
        planned: Plan,
        branch_id: &str,
        seed: u64,
        from: WorldFrom,
    ) -> Result<World> {
        let from = match from {
            WorldFrom::Base => exact(ensure_base(&hub, bench).await?),
            WorldFrom::Saves(m) => m,
        };
        World::start_parts(hub, bench, planned, branch_id, seed, Some(&from)).await
    }

    fn new(
        hub: Arc<Hub>,
        bench: &str,
        bench_client: Option<Arc<Client>>,
        tools: Vec<(ToolSchema, Route)>,
    ) -> World {
        World {
            hub,
            bench: bench.to_string(),
            bench_client,
            parts: vec![],
            tools,
            connections: Map::new(),
            track: Mutex::default(),
            started: Instant::now(),
        }
    }

    /// Starts every part (from `from`, or empty when None) and connects the ones that can be.
    /// On a failure the parts already started are stopped.
    async fn start_parts(
        hub: Arc<Hub>,
        bench: &BenchDescribe,
        planned: Plan,
        branch_id: &str,
        seed: u64,
        from: Option<&StartSaves>,
    ) -> Result<World> {
        let Plan { parts, tools } = planned;
        let bench_client = hub.client(&bench.name).await?;
        let mut world = World::new(hub, &bench.name, Some(bench_client), tools);
        for p in parts {
            if let Err(e) = world.add_part(p, branch_id, seed, from).await {
                world.stop().await;
                return Err(e);
            }
        }
        Ok(world)
    }

    async fn add_part(
        &mut self,
        p: PlannedPart,
        branch_id: &str,
        seed: u64,
        from: Option<&StartSaves>,
    ) -> Result<()> {
        let instance = instance_id(branch_id, &p.part.part);
        let (params, save) = start_params(&p.part, &instance, seed, from)?;
        let client = self.hub.client(&p.part.integration).await?;
        let part = Part {
            name: p.part.part,
            integration: p.part.integration,
            instance,
            client,
            describe: p.describe,
        };
        let _: Value = call(&part.client, &part.label(), "start", params)
            .await
            .with_context(|| format!("starting the part {}", part.name))?;
        if let Some(save) = save {
            self.track
                .get_mut()
                .unwrap()
                .last
                .insert(part.name.clone(), save);
        }
        // Pushed first: a part that started is stopped with the world even if connecting fails.
        self.parts.push(part);
        let part = &self.parts[self.parts.len() - 1];
        if let Some(info) = connect(part).await? {
            self.connections.insert(part.name.clone(), info);
        }
        Ok(())
    }

    pub fn bench(&self) -> &str {
        &self.bench
    }

    fn bench_label(&self) -> String {
        format!("the {} benchmark", self.bench)
    }

    fn bench_client(&self) -> Result<&Client, WorldError> {
        self.bench_client.as_deref().ok_or_else(|| WorldError {
            message: "no benchmark program".into(),
            fatal: true,
        })
    }

    /// Milliseconds on the world's clock (real time since the world started).
    pub fn clock_ms(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }

    /// The tools agents get (MCP shape plus `effect`).
    pub fn tools(&self) -> impl Iterator<Item = &ToolSchema> {
        self.tools.iter().map(|(t, _)| t)
    }

    pub fn is_write(&self, name: &str) -> bool {
        self.tools()
            .any(|t| t.name == name && t.effect == Effect::Write)
    }

    /// The instance id of `part`.
    pub fn instance(&self, part: &str) -> Option<&str> {
        self.part(part).ok().map(|p| p.instance.as_str())
    }

    /// The part names, in world order.
    pub fn part_names(&self) -> Vec<String> {
        self.parts.iter().map(|p| p.name.clone()).collect()
    }

    /// Each part's integration `fingerprint`.
    pub fn fingerprints(&self) -> Map<String, Value> {
        self.parts
            .iter()
            .map(|p| (p.name.clone(), p.describe.fingerprint.clone()))
            .collect()
    }

    /// The save that holds each part's current state (the start saves until a write is saved).
    pub fn saves(&self) -> Saves {
        self.track.lock().unwrap().last.clone()
    }

    fn part(&self, name: &str) -> Result<&Part, WorldError> {
        self.parts
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| WorldError::other(format!("there is no part {name} in this world")))
    }

    /// Runs a tool: the benchmark's own, or the part's that lists it. An unknown tool is a tool error.
    pub async fn call_tool(
        &self,
        name: &str,
        args: Value,
        clock_ms: i64,
    ) -> Result<Called, WorldError> {
        match self
            .tools
            .iter()
            .find(|(t, _)| t.name == name)
            .map(|(_, r)| *r)
        {
            None => Ok(Called {
                output: ToolOutput {
                    error: Some(format!("unknown tool {name}")),
                    ..Default::default()
                },
                part: None,
            }),
            Some(Route::Bench) => self.call_bench_tool(name, args).await,
            Some(Route::Part(i)) => {
                let p = &self.parts[i];
                let params = json!({"instance": p.instance, "name": name, "args": args, "clock_ms": clock_ms});
                let output: ToolOutput = call(&p.client, &p.label(), "call_tool", params).await?;
                Ok(Called {
                    output,
                    part: Some(p.name.clone()),
                })
            }
        }
    }

    async fn call_bench_tool(&self, name: &str, args: Value) -> Result<Called, WorldError> {
        let params = json!({"name": name, "args": args, "connections": self.connections});
        let o: BenchToolOutput = call(
            self.bench_client()?,
            &self.bench_label(),
            "call_tool",
            params,
        )
        .await?;
        if self.part(&o.part).is_err() {
            let error = Some(format!("the benchmark named an unknown part {}", o.part));
            return Ok(Called {
                output: ToolOutput {
                    error,
                    ..Default::default()
                },
                part: None,
            });
        }
        let output = ToolOutput {
            result: o.result,
            error: o.error,
            reads: o.reads,
            writes: o.writes,
        };
        Ok(Called {
            output,
            part: Some(o.part),
        })
    }

    async fn save_as(&self, p: &Part, save: &str) -> Result<(), WorldError> {
        let dir = self.hub.save_dir(&p.integration, save);
        let _: Value = call(
            &p.client,
            &p.label(),
            "save",
            json!({"instance": p.instance, "save": save, "dir": dir}),
        )
        .await?;
        Ok(())
    }

    /// Saves `p` under a new id and records it as the part's current state.
    async fn save_new(&self, p: &Part) -> Result<String, WorldError> {
        let save = new_save_id();
        let saved = self.save_as(p, &save).await;
        let mut t = self.track.lock().unwrap();
        match saved {
            Ok(()) => {
                t.last.insert(p.name.clone(), save.clone());
                t.dirty.remove(&p.name);
                Ok(save)
            }
            Err(e) => {
                t.dirty.insert(p.name.clone());
                Err(e)
            }
        }
    }

    /// Saves the parts that changed with a call: the part it wrote to, if any. Returns the new
    /// saves (`{}` when nothing changed). A part whose save fails stays unsaved until its next one.
    pub async fn save_changed(&self, written_part: Option<&str>) -> Result<Saves, WorldError> {
        let Some(name) = written_part else {
            return Ok(Saves::new());
        };
        let save = self.save_new(self.part(name)?).await?;
        Ok(Saves::from([(name.to_string(), save)]))
    }

    /// A save of every part's current state: its last save when nothing changed since, else a new one.
    pub async fn save_all(&self) -> Result<Saves, WorldError> {
        let mut out = Saves::new();
        for p in &self.parts {
            let reuse = {
                let t = self.track.lock().unwrap();
                t.last
                    .get(&p.name)
                    .filter(|_| !t.dirty.contains(&p.name))
                    .cloned()
            };
            let save = match reuse {
                Some(s) => s,
                None => self.save_new(p).await?,
            };
            out.insert(p.name.clone(), save);
        }
        Ok(out)
    }

    /// Saves every part under the name `names` gives it.
    async fn save_all_as(&self, names: &Saves) -> Result<(), WorldError> {
        for p in &self.parts {
            self.save_as(p, &names[&p.name]).await?;
        }
        Ok(())
    }

    /// Keys that differ between each part's live instance and `other[part]`, sorted; prefixed
    /// `part:` only when the world has more than one part.
    pub async fn diff_with(&self, other: &Saves) -> Result<Vec<String>, WorldError> {
        let many = self.parts.len() > 1;
        let mut out = vec![];
        for p in &self.parts {
            let save = other.get(&p.name).ok_or_else(|| {
                WorldError::other(format!(
                    "there is no save of the part {} to compare with",
                    p.name
                ))
            })?;
            let params = json!({"a": {"instance": p.instance}, "b": {"save": save}});
            // A malformed answer is an error, never "nothing differs".
            let d: Differing = call(&p.client, &p.label(), "diff", params).await?;
            out.extend(d.differing.into_iter().map(|k| {
                if many {
                    format!("{}:{k}", p.name)
                } else {
                    k
                }
            }));
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Every part's instance, and marks the world stopped; empty when it already was.
    fn take_instances(&self) -> Vec<Instance> {
        let mut t = self.track.lock().unwrap();
        if std::mem::replace(&mut t.stopped, true) {
            return vec![];
        }
        self.parts
            .iter()
            .map(|p| Instance {
                integration: p.integration.clone(),
                id: p.instance.clone(),
            })
            .collect()
    }

    /// Stops every instance (once). A program that has stopped is started again for this, so a
    /// crashed part's instance is still cleaned up. Failures are logged.
    pub async fn stop(&self) {
        stop_instances(&self.hub, self.take_instances()).await;
    }
}

/// `start` params of a part, and the save it starts from (None for an empty start).
fn start_params(
    part: &WorldPart,
    instance: &str,
    seed: u64,
    from: Option<&StartSaves>,
) -> Result<(Value, Option<String>)> {
    let mut params =
        json!({"instance": instance, "config": part.config, "seed": seed, "clock_ms": 0});
    let Some(from) = from else {
        return Ok((params, None));
    };
    let name = &part.part;
    let (save, mode) = from
        .get(name)
        .ok_or_else(|| anyhow!("there is no save to start the part {name} from"))?;
    params["from"] = json!({"save": save, "mode": mode});
    Ok((params, Some(save.clone())))
}

/// `connect` info of a part a benchmark may reach (exact saves only); None when it has none.
async fn connect(part: &Part) -> Result<Option<Value>> {
    if part.describe.restore.exact != ExactKind::Save {
        return Ok(None);
    }
    match part
        .client
        .call::<Value>("connect", json!({"instance": part.instance}))
        .await
    {
        Ok(info) => Ok(Some(info)),
        Err(CallError::Part { code: -32601, .. }) => Ok(None),
        Err(CallError::Stopped(tail)) => bail!(stopped_message(&part.label(), &tail)),
        Err(e) => bail!("connecting to the part {}: {e}", part.name),
    }
}

/// A running instance and the integration that runs it.
struct Instance {
    integration: String,
    id: String,
}

async fn stop_instances(hub: &Hub, instances: Vec<Instance>) {
    for Instance {
        integration,
        id: instance,
    } in instances
    {
        let r = async {
            hub.client(&integration)
                .await?
                .call::<Value>("stop", json!({"instance": instance}))
                .await
                .map_err(anyhow::Error::from)
        };
        if let Err(e) = r.await {
            eprintln!("world: could not stop {instance} ({integration}): {e:#}");
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // Best effort: an instance must not outlive its world.
        let todo = self.take_instances();
        if todo.is_empty() {
            return;
        }
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let hub = self.hub.clone();
            rt.spawn(async move { stop_instances(&hub, todo).await });
        }
    }
}
