//! Example part: one integer per instance. The core's tests spawn this program.
//!
//! `config.flavour` picks how it restores (default "exact"):
//! - "exact": exact save, no replay.
//! - "replay_only": replay only; `save` fails with -32001.
//! - "approx_and_replay": replay, plus approximate saves (reported `exact: false`).
//! - "broken_restore": exact save, but a restore from a save gives 0 (a bug on purpose).
//! - "nondeterministic": replay, but `add` adds a random 0..1000 from the OS (a bug on purpose).
use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;

use arena_part::proto::*;
use arena_part::{serve, Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

const KEY: &str = "counter/value";

#[derive(Clone, Copy, PartialEq)]
enum Flavour {
    Exact,
    ReplayOnly,
    ApproxAndReplay,
    BrokenRestore,
    Nondeterministic,
}

impl Flavour {
    fn parse(s: &str) -> Result<Flavour, PartError> {
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
    flavour: Flavour,
}

struct Saved1 {
    state: State,
    dir: String,
}

#[derive(Default)]
pub struct Counter {
    instances: Mutex<HashMap<String, Inst>>,
    saves: Mutex<HashMap<String, Saved1>>,
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
    fn value_of(&self, t: &Target) -> Result<i64, PartError> {
        match t {
            Target::Instance { instance } => self
                .instances
                .lock()
                .unwrap()
                .get(instance)
                .map(|i| i.state.value)
                .ok_or_else(|| PartError::unknown(format!("unknown instance {instance}"))),
            Target::Save { save } => self
                .saves
                .lock()
                .unwrap()
                .get(save)
                .map(|s| s.state.value)
                .ok_or_else(|| PartError::unknown(format!("unknown save {save}"))),
        }
    }
}

#[async_trait]
impl Part for Counter {
    async fn describe(&self) -> Result<Describe, PartError> {
        Ok(Describe {
            name: "counter".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: true,
            fingerprint: json!({"example": "counter"}),
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
        let flavour = Flavour::parse(
            p.config
                .get("flavour")
                .and_then(Value::as_str)
                .unwrap_or("exact"),
        )?;
        let mut state = State {
            value: 0,
            secs: p.clock_ms / 1000,
        };
        if let Some(from) = p.from {
            let exact_ok = matches!(flavour, Flavour::Exact | Flavour::BrokenRestore);
            let approx_ok = flavour == Flavour::ApproxAndReplay;
            let ok = match from.mode {
                Mode::Exact => exact_ok,
                Mode::Approximate => exact_ok || approx_ok,
            };
            if !ok {
                return Err(PartError::cant_restore(
                    "this flavour can't restore from a save in that mode",
                ));
            }
            let saves = self.saves.lock().unwrap();
            let s = saves
                .get(&from.save)
                .ok_or_else(|| PartError::unknown(format!("unknown save {}", from.save)))?;
            state = s.state;
            if flavour == Flavour::BrokenRestore {
                state.value = 0;
            }
        }
        self.instances
            .lock()
            .unwrap()
            .insert(p.instance, Inst { state, flavour });
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let mut m = self.instances.lock().unwrap();
        let inst = m
            .get_mut(&p.instance)
            .ok_or_else(|| PartError::unknown(format!("unknown instance {}", p.instance)))?;
        match p.name.as_str() {
            "add" => {
                let Some(n) = p.args.get("n").and_then(Value::as_i64) else {
                    return Ok(ToolOutput {
                        error: Some("n must be an integer".into()),
                        ..Default::default()
                    });
                };
                let extra = if inst.flavour == Flavour::Nondeterministic {
                    os_random_1000()
                } else {
                    0
                };
                inst.state.value += n + extra;
                Ok(ToolOutput {
                    result: Some(json!({"value": inst.state.value})),
                    writes: vec![KEY.into()],
                    ..Default::default()
                })
            }
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
        let (state, flavour) = {
            let m = self.instances.lock().unwrap();
            let i = m
                .get(&p.instance)
                .ok_or_else(|| PartError::unknown(format!("unknown instance {}", p.instance)))?;
            (i.state, i.flavour)
        };
        if flavour == Flavour::ReplayOnly {
            return Err(PartError::cant_restore("this flavour has no saves"));
        }
        let body = serde_json::to_vec(&json!({"value": state.value, "secs": state.secs}))
            .map_err(|e| PartError::internal(e.to_string()))?;
        std::fs::create_dir_all(&p.dir).map_err(|e| PartError::internal(e.to_string()))?;
        std::fs::write(std::path::Path::new(&p.dir).join("state.json"), &body)
            .map_err(|e| PartError::internal(e.to_string()))?;
        self.saves
            .lock()
            .unwrap()
            .insert(p.save, Saved1 { state, dir: p.dir });
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
        if let Some(s) = self.saves.lock().unwrap().remove(&p.save) {
            let _ = std::fs::remove_dir_all(s.dir);
        }
        Ok(())
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        self.instances.lock().unwrap().remove(&p.instance);
        Ok(())
    }

    async fn usage(&self) -> Result<Usage, PartError> {
        let n = self.saves.lock().unwrap().len() as u64;
        Ok(Usage { bytes: n * 32 })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve(Counter::default()).await
}
