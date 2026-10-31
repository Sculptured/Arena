//! Example part: one integer per instance. The core's tests spawn this program.
//!
//! `counter --flavour <name>` picks how it restores (default "exact"); a manifest's `run.command`
//! chooses it, and `describe` reports accordingly:
//! - "exact": exact save, no replay.
//! - "replay_only": replay only; `save` fails with -32001.
//! - "approx_and_replay": replay, plus approximate saves (reported `exact: false`).
//! - "broken_restore": exact save, but a restore from a save gives 0 (a bug on purpose).
//! - "nondeterministic": replay, but `add` adds a random 0..1000 from the OS (a bug on purpose).
//!
//! `--fingerprint <s>` adds `"machine": s` to the fingerprint, as if it ran on another machine.
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use arena_part::proto::*;
use arena_part::{serve, Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

const KEY: &str = "counter/value";

#[derive(Clone, Copy, PartialEq)]
pub enum Flavour {
    Exact,
    ReplayOnly,
    ApproxAndReplay,
    BrokenRestore,
    Nondeterministic,
}

impl Flavour {
    pub fn parse(s: &str) -> Result<Flavour, PartError> {
        Ok(match s {
            "exact" => Flavour::Exact,
            "replay_only" => Flavour::ReplayOnly,
            "approx_and_replay" => Flavour::ApproxAndReplay,
            "broken_restore" => Flavour::BrokenRestore,
            "nondeterministic" => Flavour::Nondeterministic,
            o => return Err(PartError::invalid(format!("unknown flavour: {o}"))),
        })
    }
}

#[derive(Clone, Copy)]
struct State {
    value: i64,
    /// Whole seconds of clock already counted.
    secs: i64,
}

struct Inst {
    state: State,
}

pub struct Counter {
    flavour: Flavour,
    fingerprint: Value,
    instances: Mutex<HashMap<String, Inst>>,
}

impl Default for Counter {
    fn default() -> Self {
        Counter::new(Flavour::Exact)
    }
}

/// Saves live at `$ARENA_SAVES/counter/<save>/` by convention.
fn saves_root() -> PathBuf {
    let root = std::env::var("ARENA_SAVES").unwrap_or_else(|_| "saves".into());
    PathBuf::from(root).join("counter")
}

fn read_save(save: &str) -> Result<State, PartError> {
    let path = saves_root().join(save).join("state.json");
    let body =
        std::fs::read(&path).map_err(|_| PartError::unknown(format!("unknown save {save}")))?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| PartError::internal(e.to_string()))?;
    Ok(State {
        value: v["value"].as_i64().unwrap_or(0),
        secs: v["secs"].as_i64().unwrap_or(0),
    })
}

fn os_random_1000() -> i64 {
    let mut b = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    (u32::from_le_bytes(b) % 1000) as i64
}

fn tool(name: &str, description: &str, props: Value, effect: Effect) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": props}),
        effect,
    }
}

impl Counter {
    pub fn new(flavour: Flavour) -> Counter {
        Counter {
            flavour,
            fingerprint: json!({"example": "counter"}),
            instances: Mutex::new(HashMap::new()),
        }
    }

    /// The same counter reporting `machine` in its fingerprint.
    pub fn on_machine(mut self, machine: &str) -> Counter {
        self.fingerprint["machine"] = json!(machine);
        self
    }

    /// The `add` tool: add `args.n` (plus noise, when nondeterministic) to the counter.
    fn add(&self, inst: &mut Inst, args: &Value) -> ToolOutput {
        let Some(n) = args.get("n").and_then(Value::as_i64) else {
            return ToolOutput {
                error: Some("n must be an integer".into()),
                ..Default::default()
            };
        };
        let extra = if self.flavour == Flavour::Nondeterministic {
            os_random_1000()
        } else {
            0
        };
        inst.state.value += n + extra;
        ToolOutput {
            result: Some(json!({"value": inst.state.value})),
            writes: vec![KEY.into()],
            ..Default::default()
        }
    }

    fn value_of(&self, t: &Target) -> Result<i64, PartError> {
        match t {
            Target::Instance { instance } => self
                .instances
                .lock()
                .unwrap()
                .get(instance)
                .map(|i| i.state.value)
                .ok_or_else(|| PartError::unknown(format!("unknown instance {instance}"))),
            Target::Save { save } => read_save(save).map(|s| s.value),
        }
    }
}

fn restore_of(flavour: Flavour) -> Restore {
    let (exact, approximate) = match flavour {
        Flavour::Exact | Flavour::BrokenRestore => (ExactKind::Save, false),
        Flavour::ReplayOnly | Flavour::Nondeterministic => (ExactKind::Replay, false),
        Flavour::ApproxAndReplay => (ExactKind::Replay, true),
    };
    Restore { exact, approximate }
}

/// The state a start from `from` begins with, if this flavour can restore in that mode.
fn restore_state(flavour: Flavour, from: &FromSave) -> Result<State, PartError> {
    let exact_ok = matches!(flavour, Flavour::Exact | Flavour::BrokenRestore);
    let ok = match from.mode {
        Mode::Exact => exact_ok,
        Mode::Approximate => exact_ok || flavour == Flavour::ApproxAndReplay,
    };
    if !ok {
        return Err(PartError::cant_restore(
            "this flavour can't restore from a save in that mode",
        ));
    }
    let mut state = read_save(&from.save)?;
    if flavour == Flavour::BrokenRestore {
        state.value = 0;
    }
    Ok(state)
}

#[async_trait]
impl Part for Counter {
    async fn describe(&self) -> Result<Describe, PartError> {
        Ok(Describe {
            name: "counter".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: restore_of(self.flavour),
            uses_clock: true,
            fingerprint: self.fingerprint.clone(),
            tools: vec![
                tool(
                    "add",
                    "Add n to the counter.",
                    json!({"n": {"type": "integer"}}),
                    Effect::Write,
                ),
                tool("get", "Read the counter.", json!({}), Effect::Read),
            ],
        })
    }

    async fn start(&self, p: StartParams) -> Result<(), PartError> {
        let mut state = State {
            value: 0,
            secs: p.clock_ms / 1000,
        };
        if let Some(from) = p.from {
            state = restore_state(self.flavour, &from)?;
        }
        self.instances
            .lock()
            .unwrap()
            .insert(p.instance, Inst { state });
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let mut m = self.instances.lock().unwrap();
        let inst = m
            .get_mut(&p.instance)
            .ok_or_else(|| PartError::unknown(format!("unknown instance {}", p.instance)))?;
        match p.name.as_str() {
            "add" => Ok(self.add(inst, &p.args)),
            "get" => Ok(ToolOutput {
                result: Some(json!({"value": inst.state.value})),
                reads: vec![KEY.into()],
                ..Default::default()
            }),
            o => Ok(ToolOutput {
                error: Some(format!("unknown tool {o}")),
                ..Default::default()
            }),
        }
    }

    async fn advance(&self, p: AdvanceParams) -> Result<Advanced, PartError> {
        let mut m = self.instances.lock().unwrap();
        let inst = m
            .get_mut(&p.instance)
            .ok_or_else(|| PartError::unknown(format!("unknown instance {}", p.instance)))?;
        let secs = p.clock_ms / 1000;
        let d = secs - inst.state.secs;
        if d > 0 {
            inst.state.value += d;
            inst.state.secs = secs;
        }
        Ok(Advanced {
            changed: d > 0,
            events: vec![],
        })
    }

    async fn save(&self, p: SaveParams) -> Result<Saved, PartError> {
        let flavour = self.flavour;
        let state = {
            let m = self.instances.lock().unwrap();
            let i = m
                .get(&p.instance)
                .ok_or_else(|| PartError::unknown(format!("unknown instance {}", p.instance)))?;
            i.state
        };
        if flavour == Flavour::ReplayOnly {
            return Err(PartError::cant_restore("this flavour has no saves"));
        }
        let body = serde_json::to_vec(&json!({"value": state.value, "secs": state.secs}))
            .map_err(|e| PartError::internal(e.to_string()))?;
        std::fs::create_dir_all(&p.dir).map_err(|e| PartError::internal(e.to_string()))?;
        std::fs::write(std::path::Path::new(&p.dir).join("state.json"), &body)
            .map_err(|e| PartError::internal(e.to_string()))?;
        Ok(Saved {
            bytes: body.len() as u64,
            exact: flavour != Flavour::ApproxAndReplay,
        })
    }

    async fn inspect(&self, p: InspectParams) -> Result<Docs, PartError> {
        let v = self.value_of(&p.target)?;
        let mut docs = serde_json::Map::new();
        if p.keys.is_empty() {
            docs.insert(KEY.into(), json!(v));
        }
        for k in &p.keys {
            docs.insert(k.clone(), if k == KEY { json!(v) } else { Value::Null });
        }
        Ok(Docs { docs })
    }

    async fn diff(&self, p: DiffParams) -> Result<Differing, PartError> {
        let (a, b) = (self.value_of(&p.a)?, self.value_of(&p.b)?);
        Ok(Differing {
            differing: if a == b { vec![] } else { vec![KEY.into()] },
        })
    }

    async fn drop_save(&self, p: SaveId) -> Result<(), PartError> {
        let _ = std::fs::remove_dir_all(saves_root().join(&p.save));
        Ok(())
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        self.instances.lock().unwrap().remove(&p.instance);
        Ok(())
    }

    async fn usage(&self) -> Result<Usage, PartError> {
        // Every save lives in ARENA_SAVES, which arenad measures itself: nothing else is kept.
        Ok(Usage { bytes: 0 })
    }
}

/// The value after `--<name>` on the command line.
fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    Some(args.get(i + 1).map(String::as_str).unwrap_or(""))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flavour = match arg(&args, "--flavour") {
        Some(f) => Flavour::parse(f)?,
        None => Flavour::Exact,
    };
    let mut counter = Counter::new(flavour);
    if let Some(machine) = arg(&args, "--fingerprint") {
        counter = counter.on_machine(machine);
    }
    serve(counter).await
}
