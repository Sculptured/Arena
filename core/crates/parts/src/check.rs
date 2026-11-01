//! The conformance check behind `arena-part-check` (docs/protocol.md, "Determinism contract").
use crate::client::{CallError, Client};
use crate::hub::Hub;
use arena_part::proto::{Describe, Effect, ExactKind, PROTOCOL};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// The steps in order, for naming those a failure kept from running.
const STEPS: &[&str] = &[
    "describe",
    "start",
    "examples",
    "save_1",
    "write_example",
    "save_2",
    "restore",
    "diff_self",
    "replay",
    "stop",
    "drop_saves",
];
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

/// The tool and arguments of an `[[example]]` entry.
fn example_call(ex: &Value) -> Res<(String, Value)> {
    let tool = ex["tool"].as_str().ok_or("an example has no `tool`")?;
    let args = ex.get("args").cloned().unwrap_or_else(|| json!({}));
    Ok((tool.to_string(), args))
}

/// The `reads` and `writes` keys of a tool output.
fn touched_keys(out: &Value) -> impl Iterator<Item = &str> {
    ["reads", "writes"].into_iter().flat_map(|k| {
        out[k]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    })
}

impl Run<'_> {
    async fn call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        self.client.call(method, params).await
    }

    async fn call_tool(&self, inst: &str, name: &str, args: &Value) -> Result<Value, CallError> {
        self.call(
            "call_tool",
            json!({"instance": inst, "name": name, "args": args, "clock_ms": 0}),
        )
        .await
    }

    async fn advance(&self, inst: &str, clock_ms: i64) -> Result<Value, CallError> {
        self.call("advance", json!({"instance": inst, "clock_ms": clock_ms}))
            .await
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
            .call_tool("chk_a", name, args)
            .await
            .map_err(|e| format!("{name}: {e}"))?;
        if let Some(e) = out.get("error").filter(|e| !e.is_null()) {
            return Err(format!("{name} returned an error: {e}"));
        }
        self.keys.extend(touched_keys(&out).map(str::to_string));
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
        let bad = || format!("diff gave no `differing` list of keys: {out}");
        out["differing"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|k| k.as_str().map(str::to_string).ok_or_else(bad))
            .collect()
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

    /// Records the step's outcome, with `detail` of its value when it passed.
    fn step_value<T>(
        &mut self,
        name: &str,
        r: Res<T>,
        detail: impl FnOnce(&T) -> String,
    ) -> Res<T> {
        let (ok, detail) = match &r {
            Ok(v) => (true, detail(v)),
            Err(e) => (false, e.clone()),
        };
        self.report.steps.push(Step {
            name: name.into(),
            ok,
            detail,
        });
        r
    }

    fn step(&mut self, name: &str, r: Res<String>) -> Res<()> {
        self.step_value(name, r, String::clone).map(|_| ())
    }

    fn skip(&mut self, msg: &str) {
        self.report.skipped.push(msg.into());
    }

    async fn run(&mut self, config: &Value) -> Res<()> {
        let m = self.hub.registry().get(&self.package).unwrap().clone();
        let d = self.describe_step().await;
        let d = self.step_value("describe", d, |d| format!("{} tools", d.tools.len()))?;
        let r = self.start_step(config).await;
        let usage_before = self.step_value("start", r, |_| "chk_a started".into())?;
        let r = self.examples_step(&m.examples, &d).await;
        self.step("examples", r)?;
        let r = self.save("chk_s1", &d).await;
        let have_saves = self.step_value("save_1", r, |saved| {
            if *saved { "saved chk_s1" } else { "no saves" }.into()
        })?;
        self.write_and_second_save(&m.examples, &d, have_saves)
            .await?;
        self.restore_and_self_diff(&d, config, have_saves).await?;
        self.replay_phase(&d, config).await?;
        let r = self.stop_all_step().await;
        self.step("stop", r)?;
        let r = self.drop_saves_step(usage_before).await;
        self.step("drop_saves", r)
    }

    /// 1. describe, and check what it says.
    async fn describe_step(&self) -> Res<Describe> {
        let d = self
            .hub
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

    /// 2. start `chk_a`; the usage is read first, to compare after the saves are dropped.
    async fn start_step(&mut self, config: &Value) -> Res<u64> {
        let usage_before = self.usage().await?;
        self.start("chk_a", config, None).await?;
        Ok(usage_before)
    }

    /// 3. run every example.
    async fn examples_step(&mut self, examples: &[Value], d: &Describe) -> Res<String> {
        if examples.is_empty() {
            return Err("no [[example]] entries; the check needs at least one".to_string());
        }
        for ex in examples {
            let (tool, args) = example_call(ex)?;
            if !d.tools.iter().any(|t| t.name == tool) {
                return Err(format!("example tool {tool:?} is not in describe"));
            }
            self.tool(&tool, &args).await?;
        }
        Ok(format!("{} examples ran", examples.len()))
    }

    /// 5 and 6. run a write example (and the clock), then save `chk_s2`.
    async fn write_and_second_save(
        &mut self,
        examples: &[Value],
        d: &Describe,
        have_saves: bool,
    ) -> Res<()> {
        let write = examples.iter().find(|ex| {
            ex["tool"]
                .as_str()
                .and_then(|n| d.tools.iter().find(|t| t.name == n))
                .is_some_and(|t| t.effect == Effect::Write)
        });
        let Some(ex) = write else {
            self.skip("write_example: no example uses a write tool; save_2 skipped too");
            return Ok(());
        };
        let r = self.write_example(ex, d).await;
        self.step("write_example", r)?;
        if have_saves {
            let r = self.save("chk_s2", d).await;
            self.step("save_2", r.map(|_| "saved chk_s2".to_string()))?;
        }
        Ok(())
    }

    /// The write example, then the clock (which is exercised before the second save, and replayed).
    async fn write_example(&mut self, ex: &Value, d: &Describe) -> Res<String> {
        let (tool, args) = example_call(ex)?;
        self.tool(&tool, &args).await?;
        if d.uses_clock {
            self.advance("chk_a", ADVANCE_MS).await.map_err(show)?;
            self.inputs.push(Input::Advance(ADVANCE_MS));
        }
        Ok(format!("ran {tool}"))
    }

    /// 7 and 8. restore in each supported mode, and diff a save with itself.
    async fn restore_and_self_diff(
        &mut self,
        d: &Describe,
        config: &Value,
        have_saves: bool,
    ) -> Res<()> {
        if !have_saves {
            self.skip("restore: no saves to restore from");
            self.skip("diff_self: no saves");
            return Ok(());
        }
        let r = self.restore_step(d, config).await;
        self.step("restore", r)?;
        let r = self.diff_self_step().await;
        self.step("diff_self", r)
    }

    async fn diff_self_step(&self) -> Res<String> {
        let v = self
            .differing(json!({"save": "chk_s1"}), json!({"save": "chk_s1"}))
            .await?;
        if v.is_empty() {
            Ok("empty".to_string())
        } else {
            Err(format!("diff of a save with itself lists {v:?}"))
        }
    }

    /// 9. replay the inputs on a fresh instance, for a part that restores by replay.
    async fn replay_phase(&mut self, d: &Describe, config: &Value) -> Res<()> {
        if d.restore.exact != ExactKind::Replay {
            self.skip("replay: the part restores from saves");
            return Ok(());
        }
        let have_s2 = self.saves.contains(&"chk_s2");
        let r = self.replay_step(config, have_s2).await;
        self.step("replay", r)
    }

    /// 10. stop everything (a part may free an instance's storage on stop).
    async fn stop_all_step(&mut self) -> Res<String> {
        for i in self.live.clone() {
            self.stop(i).await?;
        }
        Ok("all instances stopped".to_string())
    }

    /// 11. drop saves; with every instance stopped, usage must be back to where it started.
    async fn drop_saves_step(&mut self, usage_before: u64) -> Res<String> {
        while let Some(s) = self.saves.first().copied() {
            self.call("drop_save", json!({"save": s}))
                .await
                .map_err(|e| format!("drop_save {s}: {e}"))?;
            self.saves.remove(0);
        }
        let after = self.usage().await?;
        if after > usage_before + USAGE_ALLOWANCE {
            return Err(format!(
                "usage grew from {usage_before} to {after} bytes after stopping the instances and dropping the saves"
            ));
        }
        Ok(format!("usage {usage_before} -> {after} bytes"))
    }

    /// What the part keeps: its `usage` (outside the save storage) plus its save folders.
    async fn usage(&self) -> Res<u64> {
        let v = self
            .call("usage", json!({}))
            .await
            .map_err(|e| format!("usage: {e}"))?;
        let own = v["bytes"]
            .as_u64()
            .ok_or_else(|| "usage gave no bytes".to_string())?;
        Ok(own + self.hub.package_saves_bytes(&self.package))
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
            let diff = self.restore_diff(config, mode).await;
            notes.push(restore_note(mode, diff)?);
            if self.live.contains(&"chk_b") {
                self.stop("chk_b").await?;
            }
        }
        Ok(notes.join("; "))
    }

    /// Start `chk_b` from `chk_s1` in `mode`; the keys in which it differs from the save.
    async fn restore_diff(&mut self, config: &Value, mode: &str) -> Res<Vec<String>> {
        self.start("chk_b", config, Some(("chk_s1", mode))).await?;
        self.differing(json!({"instance": "chk_b"}), json!({"save": "chk_s1"}))
            .await
    }

    async fn replay_step(&mut self, config: &Value, have_s2: bool) -> Res<String> {
        self.start("chk_r", config, None).await?;
        self.replay_inputs().await?;
        let outcome = if have_s2 {
            self.compare_replay_with_save().await?
        } else {
            self.compare_replay_by_inspect().await?
        };
        self.stop("chk_r").await?;
        Ok(outcome.into())
    }

    /// Send the recorded inputs to `chk_r`; each result must be the one seen the first time.
    async fn replay_inputs(&self) -> Res<()> {
        for i in &self.inputs {
            match i {
                Input::Call { name, args, result } => {
                    let out = self
                        .call_tool("chk_r", name, args)
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
                    self.advance("chk_r", *ms)
                        .await
                        .map_err(|e| format!("replay advance: {e}"))?;
                }
            }
        }
        Ok(())
    }

    async fn compare_replay_with_save(&self) -> Res<&'static str> {
        let v = self
            .differing(json!({"instance": "chk_r"}), json!({"save": "chk_s2"}))
            .await?;
        if !v.is_empty() {
            return Err(format!("replay differs from chk_s2 in {v:?}"));
        }
        Ok("replay matches chk_s2")
    }

    /// Without a second save: inspect the keys the examples touched, in both instances.
    async fn compare_replay_by_inspect(&self) -> Res<&'static str> {
        let keys: Vec<&String> = self.keys.iter().collect();
        if keys.is_empty() {
            return Err("no keys reported by the examples, so the replay can't be compared".into());
        }
        let mut docs = vec![];
        for inst in ["chk_r", "chk_a"] {
            docs.push(self.inspect_docs(inst, &keys).await?);
        }
        if docs[0] != docs[1] {
            return Err(format!(
                "replay differs from the first run: {} against {}",
                docs[0], docs[1]
            ));
        }
        Ok("replay matches the first run by inspect (no saves)")
    }

    async fn inspect_docs(&self, inst: &str, keys: &[&String]) -> Res<Value> {
        let v = self
            .call(
                "inspect",
                json!({"target": {"instance": inst}, "keys": keys}),
            )
            .await
            .map_err(|e| format!("inspect {inst}: {e}"))?;
        match v["docs"].as_object() {
            Some(m) if !m.is_empty() => Ok(v["docs"].clone()),
            _ => Err(format!("inspect {inst} gave no docs: {v}")),
        }
    }
}

/// What a restore in `mode` says: exact must match (an error otherwise); approximate is
/// reported, not failed.
fn restore_note(mode: &str, diff: Res<Vec<String>>) -> Res<String> {
    if mode == "exact" {
        let v = diff.map_err(|e| format!("exact restore: {e}"))?;
        if !v.is_empty() {
            return Err(format!("exact restore differs from the save in {v:?}"));
        }
        return Ok("exact restore matches".into());
    }
    Ok(match diff {
        Ok(v) if v.is_empty() => "approximate restore matches".to_string(),
        Ok(v) => format!("approximate restore differs in {v:?} (reported)"),
        Err(e) => format!("approximate restore failed: {e} (reported)"),
    })
}

fn empty_report(package: &str) -> Report {
    Report {
        package: package.into(),
        passed: false,
        steps: vec![],
        skipped: vec![],
    }
}

fn not_started(package: &str, err: &str) -> Report {
    let mut report = empty_report(package);
    report.steps.push(Step {
        name: "describe".into(),
        ok: false,
        detail: format!("the program did not start: {err}"),
    });
    report
}

/// Runs the conformance check against the package `name` of `hub`.
pub async fn check(hub: &Hub, name: &str, config: Value) -> Report {
    let client = match hub.client(name).await {
        Ok(c) => c,
        Err(e) => return not_started(name, &e.to_string()),
    };
    let mut run = Run {
        hub,
        package: name.into(),
        client,
        report: empty_report(name),
        live: vec![],
        saves: vec![],
        inputs: vec![],
        keys: BTreeSet::new(),
    };
    let r = run.run(&config).await;
    run.clean_up().await;
    run.note_steps_not_run();
    run.report.passed = r.is_ok() && run.report.steps.iter().all(|s| s.ok);
    run.report
}

impl Run<'_> {
    /// A failed check still leaves nothing behind.
    async fn clean_up(&mut self) {
        let mut leftovers = vec![];
        for i in self.live.clone() {
            if let Err(e) = self.stop(i).await {
                leftovers.push(format!("stop {i}: {e}"));
            }
        }
        for s in self.saves.clone() {
            if let Err(e) = self.call("drop_save", json!({"save": s})).await {
                leftovers.push(format!("drop_save {s}: {e}"));
            }
        }
        if !leftovers.is_empty() {
            self.report.steps.push(Step {
                name: "cleanup".into(),
                ok: false,
                detail: leftovers.join("; "),
            });
        }
    }

    /// Say which steps never ran.
    fn note_steps_not_run(&mut self) {
        let Some(failed) = self
            .report
            .steps
            .iter()
            .find(|s| !s.ok)
            .map(|s| s.name.clone())
        else {
            return;
        };
        for n in STEPS {
            let seen = self.report.steps.iter().any(|s| s.name == *n)
                || self.report.skipped.iter().any(|s| s.starts_with(n));
            if !seen {
                self.skip(&format!("not run: {n} (after {failed} failed)"));
            }
        }
    }
}
