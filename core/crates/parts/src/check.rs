//! The conformance check behind `arena-part-check` (docs/protocol.md, "Determinism contract").
use crate::client::{CallError, Client};
use crate::hub::Hub;
use arena_part::proto::{Describe, Effect, ExactKind, PROTOCOL};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

const SEED: u64 = 1;
/// How far a clock part is advanced before the second save, to exercise clock state.
const ADVANCE_MS: i64 = 1500;
/// Disk growth tolerated after both saves are dropped.
const USAGE_ALLOWANCE: u64 = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub package: String,
    pub passed: bool,
    pub steps: Vec<Step>,
    pub skipped: Vec<String>,
}

/// An input sent to `chk_a`, in order, for the replay.
enum Input {
    Call {
        name: String,
        args: Value,
        result: Option<Value>,
    },
    Advance(i64),
}

struct Run<'a> {
    hub: &'a Hub,
    package: String,
    client: Arc<Client>,
    report: Report,
    live: Vec<&'static str>,
    saves: Vec<&'static str>,
    inputs: Vec<Input>,
    keys: BTreeSet<String>,
}

type Res<T> = Result<T, String>;

fn show(e: CallError) -> String {
    e.to_string()
}

fn is_code(e: &CallError, code: i64) -> bool {
    matches!(e, CallError::Part { code: c, .. } if *c == code)
}

fn tool_name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Run<'_> {
    async fn call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        self.client.call(method, params).await
    }

    async fn start(
        &mut self,
        inst: &'static str,
        config: &Value,
        from: Option<(&str, &str)>,
    ) -> Res<()> {
        let mut p = json!({"instance": inst, "config": config, "seed": SEED, "clock_ms": 0});
        if let Some((save, mode)) = from {
            p["from"] = json!({"save": save, "mode": mode});
        }
        self.call("start", p).await.map_err(show)?;
        if !self.live.contains(&inst) {
            self.live.push(inst);
        }
        Ok(())
    }

    async fn stop(&mut self, inst: &'static str) -> Res<()> {
        self.call("stop", json!({"instance": inst}))
            .await
            .map_err(show)?;
        self.live.retain(|i| *i != inst);
        Ok(())
    }

    /// `call_tool` on `chk_a`, recorded for the replay.
    async fn tool(&mut self, name: &str, args: &Value) -> Res<()> {
        let out = self
            .call(
                "call_tool",
                json!({"instance": "chk_a", "name": name, "args": args, "clock_ms": 0}),
            )
            .await
            .map_err(|e| format!("{name}: {e}"))?;
        if let Some(e) = out.get("error").filter(|e| !e.is_null()) {
            return Err(format!("{name} returned an error: {e}"));
        }
        for k in ["reads", "writes"] {
            for key in out[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                self.keys.insert(key.to_string());
            }
        }
        self.inputs.push(Input::Call {
            name: name.into(),
            args: args.clone(),
            result: out.get("result").cloned(),
        });
        Ok(())
    }

    async fn differing(&self, a: Value, b: Value) -> Res<Vec<String>> {
        let out = self
            .call("diff", json!({"a": a, "b": b}))
            .await
            .map_err(show)?;
        Ok(out["differing"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect())
    }

    /// Saves `chk_a`; `Ok(false)` when the part can't save and says so with -32001.
    async fn save(&mut self, id: &'static str, d: &Describe) -> Res<bool> {
        let dir = self.hub.save_dir(&self.package, id);
        let r = self
            .call(
                "save",
                json!({"instance": "chk_a", "save": id, "dir": dir.to_string_lossy()}),
            )
            .await;
        match r {
            Ok(v) => {
                self.saves.push(id);
                if d.restore.exact == ExactKind::Save && v["exact"] != json!(true) {
                    return Err(format!(
                        "{id}: a part with exact saves reported exact: false"
                    ));
                }
                Ok(true)
            }
            Err(e) if is_code(&e, -32001) && d.restore.exact != ExactKind::Save => {
                self.report
                    .skipped
                    .push(format!("{id}: the part has no saves ({e})"));
                Ok(false)
            }
            Err(e) => Err(format!("{id}: {e}")),
        }
    }

    fn step(&mut self, name: &str, r: Res<String>) -> Res<()> {
        let (ok, detail) = match &r {
            Ok(d) => (true, d.clone()),
            Err(e) => (false, e.clone()),
        };
        self.report.steps.push(Step {
            name: name.into(),
            ok,
            detail,
        });
        r.map(|_| ())
    }

    async fn run(&mut self, config: &Value) -> Res<()> {
        let hub = self.hub;
        let m = hub.registry().get(&self.package).unwrap().clone();

        // 1. describe
        let r = async {
            let d = hub
                .describe(&self.package)
                .await
                .map_err(|e| e.to_string())?;
            if d.protocol != PROTOCOL {
                return Err(format!("protocol is {}, not {PROTOCOL}", d.protocol));
            }
            let mut seen = HashSet::new();
            for t in &d.tools {
                if !tool_name_ok(&t.name) {
                    return Err(format!("tool name {:?} is not well formed", t.name));
                }
                if !seen.insert(&t.name) {
                    return Err(format!("tool name {:?} appears twice", t.name));
                }
            }
            Ok(d)
        }
        .await;
        let d = match r {
            Ok(d) => {
                self.step("describe", Ok(format!("{} tools", d.tools.len())))?;
                d
            }
            Err(e) => return self.step("describe", Err(e)),
        };
        let usage_before = self.usage().await.unwrap_or(0);

        // 2. start chk_a
        let r = self
            .start("chk_a", config, None)
            .await
            .map(|_| "chk_a started".into());
        self.step("start", r)?;

        // 3. examples
        let r = async {
            for ex in &m.examples {
                let tool = ex["tool"]
                    .as_str()
                    .ok_or("an example has no `tool`")?
                    .to_string();
                let args = ex.get("args").cloned().unwrap_or_else(|| json!({}));
                if !d.tools.iter().any(|t| t.name == tool) {
                    return Err(format!("example tool {tool:?} is not in describe"));
                }
                self.tool(&tool, &args).await?;
            }
            Ok(format!("{} examples ran", m.examples.len()))
        }
        .await;
        self.step("examples", r)?;

        // 4. save chk_s1
        let r = self.save("chk_s1", &d).await;
        let have_saves = match r {
            Ok(b) => {
                self.step(
                    "save_1",
                    Ok(if b { "saved chk_s1" } else { "no saves" }.into()),
                )?;
                b
            }
            Err(e) => return self.step("save_1", Err(e)),
        };

        // 5. a write example
        let write = m.examples.iter().find(|ex| {
            ex["tool"]
                .as_str()
                .and_then(|n| d.tools.iter().find(|t| t.name == n))
                .is_some_and(|t| t.effect == Effect::Write)
        });
        match write {
            Some(ex) => {
                let tool = ex["tool"].as_str().unwrap().to_string();
                let args = ex.get("args").cloned().unwrap_or_else(|| json!({}));
                // The clock is exercised before the second save, and replayed.
                let r = async {
                    self.tool(&tool, &args).await?;
                    if d.uses_clock {
                        self.call(
                            "advance",
                            json!({"instance": "chk_a", "clock_ms": ADVANCE_MS}),
                        )
                        .await
                        .map_err(show)?;
                        self.inputs.push(Input::Advance(ADVANCE_MS));
                    }
                    Ok(format!("ran {tool}"))
                }
                .await;
                self.step("write_example", r)?;
                // 6. save chk_s2
                if have_saves {
                    let r = self
                        .save("chk_s2", &d)
                        .await
                        .map(|_| "saved chk_s2".to_string());
                    self.step("save_2", r)?;
                }
            }
            None => self
                .report
                .skipped
                .push("write_example: no example uses a write tool; save_2 skipped too".into()),
        }
        let have_s2 = self.saves.contains(&"chk_s2");

        // 7. restore in each supported mode
        if !have_saves {
            self.report
                .skipped
                .push("restore: no saves to restore from".into());
        } else {
            let r = self.restore_step(&d, config).await;
            self.step("restore", r)?;
        }

        // 8. diff(s1, s1)
        if have_saves {
            let r = self
                .differing(json!({"save": "chk_s1"}), json!({"save": "chk_s1"}))
                .await
                .and_then(|v| {
                    if v.is_empty() {
                        Ok("empty".to_string())
                    } else {
                        Err(format!("diff of a save with itself lists {v:?}"))
                    }
                });
            self.step("diff_self", r)?;
        } else {
            self.report.skipped.push("diff_self: no saves".into());
        }

        // 9. replay
        if d.restore.exact == ExactKind::Replay {
            let r = self.replay_step(config, have_s2).await;
            self.step("replay", r)?;
        } else {
            self.report
                .skipped
                .push("replay: the part restores from saves".into());
        }

        // 10. drop saves; usage must not grow
        let r = async {
            for s in std::mem::take(&mut self.saves) {
                self.call("drop_save", json!({"save": s}))
                    .await
                    .map_err(show)?;
            }
            let after = self.usage().await?;
            if after > usage_before + USAGE_ALLOWANCE {
                return Err(format!(
                    "usage grew from {usage_before} to {after} bytes after dropping the saves"
                ));
            }
            Ok(format!("usage {usage_before} -> {after} bytes"))
        }
        .await;
        self.step("drop_saves", r)?;

        // 11. stop everything
        let r = async {
            for i in self.live.clone() {
                self.stop(i).await?;
            }
            Ok("all instances stopped".to_string())
        }
        .await;
        self.step("stop", r)
    }

    async fn usage(&self) -> Res<u64> {
        let v = self
            .call("usage", json!({}))
            .await
            .map_err(|e| format!("usage: {e}"))?;
        v["bytes"]
            .as_u64()
            .ok_or_else(|| "usage gave no bytes".into())
    }

    async fn restore_step(&mut self, d: &Describe, config: &Value) -> Res<String> {
        let mut notes = vec![];
        let mut modes = vec![];
        if d.restore.exact == ExactKind::Save {
            modes.push("exact");
        } else {
            notes.push("exact skipped: the part restores by replay".to_string());
        }
        if d.restore.approximate {
            modes.push("approximate");
        }
        for mode in modes {
            let started = self.start("chk_b", config, Some(("chk_s1", mode))).await;
            let diff = match started {
                Ok(()) => {
                    self.differing(json!({"instance": "chk_b"}), json!({"save": "chk_s1"}))
                        .await
                }
                Err(e) => Err(e),
            };
            if mode == "exact" {
                let v = diff.map_err(|e| format!("exact restore: {e}"))?;
                if !v.is_empty() {
                    return Err(format!("exact restore differs from the save in {v:?}"));
                }
                notes.push("exact restore matches".into());
            } else {
                // Approximate is reported, not failed.
                notes.push(match diff {
                    Ok(v) if v.is_empty() => "approximate restore matches".to_string(),
                    Ok(v) => format!("approximate restore differs in {v:?} (reported)"),
                    Err(e) => format!("approximate restore failed: {e} (reported)"),
                });
            }
            if self.live.contains(&"chk_b") {
                self.stop("chk_b").await?;
            }
        }
        Ok(notes.join("; "))
    }

    async fn replay_step(&mut self, config: &Value, have_s2: bool) -> Res<String> {
        self.start("chk_r", config, None).await?;
        let inputs = std::mem::take(&mut self.inputs);
        for i in &inputs {
            match i {
                Input::Call { name, args, result } => {
                    let out = self
                        .call(
                            "call_tool",
                            json!({"instance": "chk_r", "name": name, "args": args, "clock_ms": 0}),
                        )
                        .await
                        .map_err(|e| format!("replay {name}: {e}"))?;
                    if out.get("result") != result.as_ref() {
                        return Err(format!(
                            "replay diverged: {name} gave {:?}, first time {:?}",
                            out.get("result"),
                            result
                        ));
                    }
                }
                Input::Advance(ms) => {
                    self.call("advance", json!({"instance": "chk_r", "clock_ms": ms}))
                        .await
                        .map_err(|e| format!("replay advance: {e}"))?;
                }
            }
        }
        let outcome = if have_s2 {
            let v = self
                .differing(json!({"instance": "chk_r"}), json!({"save": "chk_s2"}))
                .await?;
            if !v.is_empty() {
                return Err(format!("replay differs from chk_s2 in {v:?}"));
            }
            "replay matches chk_s2"
        } else {
            let keys: Vec<&String> = self.keys.iter().collect();
            let mut docs = vec![];
            for inst in ["chk_r", "chk_a"] {
                let v = self
                    .call(
                        "inspect",
                        json!({"target": {"instance": inst}, "keys": keys}),
                    )
                    .await
                    .map_err(|e| format!("inspect {inst}: {e}"))?;
                docs.push(v["docs"].clone());
            }
            if docs[0] != docs[1] {
                return Err(format!(
                    "replay differs from the first run: {} against {}",
                    docs[0], docs[1]
                ));
            }
            "replay matches the first run by inspect (no saves)"
        };
        self.inputs = inputs;
        self.stop("chk_r").await?;
        Ok(outcome.into())
    }
}

/// Runs the conformance check against the package `name` of `hub`.
pub async fn check(hub: &Hub, name: &str, config: Value) -> Report {
    let mut report = Report {
        package: name.into(),
        passed: false,
        steps: vec![],
        skipped: vec![],
    };
    let client = match hub.client(name).await {
        Ok(c) => c,
        Err(e) => {
            report.steps.push(Step {
                name: "describe".into(),
                ok: false,
                detail: format!("the program did not start: {e}"),
            });
            return report;
        }
    };
    let mut run = Run {
        hub,
        package: name.into(),
        client,
        report,
        live: vec![],
        saves: vec![],
        inputs: vec![],
        keys: BTreeSet::new(),
    };
    let r = run.run(&config).await;
    // A failed check still leaves nothing behind.
    for i in run.live.clone() {
        let _ = run.stop(i).await;
    }
    for s in std::mem::take(&mut run.saves) {
        let _ = run.call("drop_save", json!({"save": s})).await;
    }
    run.report.passed = r.is_ok() && run.report.steps.iter().all(|s| s.ok);
    run.report
}
