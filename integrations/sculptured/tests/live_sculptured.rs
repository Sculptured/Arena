//! Runs real `sculptured serve` instances: `cargo test -p arena-sculptured -- --ignored live_`.
//! Needs the sculptured program (`sculptured` on `PATH`, or env
//! `ARENA_SCULPTURED_BINARY`); without it the test says so and passes. Every server and folder
//! the test makes is removed afterwards, even on failure (the drop runs the integration's `gc`).
use arena_part::proto::*;
use arena_part::Part;
use arena_sculptured::Sculptured;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// The test's folder; dropping it stops every instance recorded under it and removes it.
struct World {
    dir: PathBuf,
}

impl World {
    fn new() -> World {
        let dir = std::env::temp_dir().join(format!("arena_live_sc_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        World { dir }
    }

    fn saves(&self) -> PathBuf {
        self.dir.join("saves/sculptured")
    }

    /// The integration, as a program that may have been restarted since (it keeps nothing else).
    fn part(&self) -> Sculptured {
        Sculptured::new(self.saves())
    }

    /// The processes with the test folder on their command line (`pgrep -f`; only to check).
    fn running(&self) -> String {
        let out = Command::new("pgrep")
            .arg("-af")
            .arg(&self.dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // a fresh value started nothing, so its gc stops every instance recorded here
        let part = self.part();
        let gc = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(part.gc(GcParams { keep: vec![] }))
        });
        let _ = gc.join();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn binary() -> Option<PathBuf> {
    let b = std::env::var_os("ARENA_SCULPTURED_BINARY")
        .map(PathBuf::from)
        .or_else(arena_sculptured::default_binary)?;
    b.is_file().then_some(b)
}

fn start(instance: &str, binary: &PathBuf, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config: json!({"binary": binary, "team": "arena"}),
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn tool(p: &Sculptured, instance: &str, name: &str, args: Value) -> ToolOutput {
    let params = CallToolParams {
        instance: instance.into(),
        name: name.into(),
        args,
        clock_ms: 0,
    };
    p.call_tool(params).await.unwrap()
}

/// `decide_change` with `args` on `inst`.
async fn decide(p: &Sculptured, inst: &str, args: Value) -> ToolOutput {
    tool(p, inst, "decide_change", args).await
}

async fn changes(p: &Sculptured, inst: &str) -> Value {
    let out = tool(p, inst, "list_changes", json!({})).await;
    out.result.unwrap()["changes"].clone()
}

fn instance(i: &str) -> Target {
    Target::Instance { instance: i.into() }
}

fn save(s: &str) -> Target {
    Target::Save { save: s.into() }
}

async fn differing(p: &Sculptured, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

async fn save_as(w: &World, inst: &str, id: &str) {
    let params = SaveParams {
        instance: inst.into(),
        save: id.into(),
        dir: w.saves().join(id).to_string_lossy().into(),
    };
    let saved = w.part().save(params).await.unwrap();
    assert!(saved.exact && saved.bytes > 0, "{saved:?}");
}

fn order() -> Value {
    json!({"action": "create_purchase_order", "system": "ERP",
           "summary": "Order 2 SP-118 bearings from Acme Hydraulics, $4200",
           "payload": {"item": "SP-118", "qty": 2, "supplier": "Acme Hydraulics", "amount": 4200},
           "reason": "the press is down"})
}

/// Proposes the order on `inst`; the change waits for a decision. Returns its id.
async fn propose(p: &Sculptured, inst: &str) -> String {
    let out = tool(p, inst, "propose_change", order()).await;
    let change = out.result.expect("propose_change gives the change");
    assert_eq!(change["action"], "create_purchase_order");
    assert_eq!(change["payload"]["amount"], 4200);
    assert_eq!(change["decision"], Value::Null, "{change}");
    let id = change["id"].as_str().unwrap().to_string();
    assert_eq!(out.writes, [format!("change/{id}")]);
    assert_eq!(changes(p, inst).await[0], change);
    id
}

/// A refusal is the tool's error, and it changes nothing.
fn refused(out: &ToolOutput, words: &str) {
    let e = out.error.as_deref().unwrap_or_default();
    assert!(e.contains(words), "{out:?}");
    assert!(out.result.is_none() && out.reads.is_empty() && out.writes.is_empty());
}

/// Decides change `id` on `inst` (refused with `note`, or approved) and checks the outcome.
async fn decided(p: &Sculptured, inst: &str, id: &str, note: Option<&str>) {
    let args = match note {
        Some(n) => json!({"id": id, "approved": false, "note": n}),
        None => json!({"id": id, "approved": true}),
    };
    let out = decide(p, inst, args).await;
    let change = out.result.expect("decide_change gives the change");
    assert_eq!(change["id"], id, "shown under the id it was proposed with");
    assert_eq!(change["decision"]["approved"], note.is_none(), "{change}");
    assert_eq!(change["decision"]["by"], "person");
    assert_eq!(change["decision"]["note"], json!(note));
    assert_eq!(out.writes, [format!("change/{id}")]);
}

async fn refusals(p: &Sculptured, inst: &str, id: &str) {
    let again = decide(p, inst, json!({"id": id, "approved": false})).await;
    refused(&again, "already decided: approved by person");
    let nope = decide(p, inst, json!({"id": "chg-nope", "approved": true})).await;
    refused(&nope, "no change chg-nope");
    refused(&decide(p, inst, json!({"id": id})).await, "approved");
    let bad = json!({"action": "x", "summary": "s"});
    refused(
        &tool(p, inst, "propose_change", bad).await,
        "payload must be an object",
    );
    let unknown = CallToolParams {
        instance: inst.into(),
        name: "nope".into(),
        args: json!({}),
        clock_ms: 0,
    };
    assert_eq!(p.call_tool(unknown).await.unwrap_err().code, -32602);
}

/// `inst`, started from `s`, shows change `id` waiting again, once, as in the save.
async fn waits_again(p: &Sculptured, inst: &str, s: &str, id: &str) {
    let listed = changes(p, inst).await;
    assert_eq!(
        listed.as_array().unwrap().len(),
        1,
        "no copy shown: {listed}"
    );
    assert_eq!(listed[0]["id"], id);
    assert_eq!(listed[0]["decision"], Value::Null);
    assert!(differing(p, instance(inst), save(s)).await.is_empty());
    let keys = vec![format!("change/{id}"), "change/nope".into()];
    let target = instance(inst);
    let docs = p
        .inspect(InspectParams { target, keys })
        .await
        .unwrap()
        .docs;
    assert_eq!(docs[&format!("change/{id}")], listed[0]);
    assert_eq!(docs["change/nope"], Value::Null);
}

/// Two restores of `s1` decide `id` differently; a restore of a save of a restore decides it too.
async fn restores_decide(w: &World, p: &Sculptured, binary: &PathBuf, id: &str) {
    p.start(start("t14_b", binary, Some("s1"))).await.unwrap();
    waits_again(p, "t14_b", "s1", id).await;
    decided(p, "t14_b", id, Some("too dear")).await;
    p.start(start("t14_c", binary, Some("s1"))).await.unwrap();
    waits_again(p, "t14_c", "s1", id).await;
    save_as(w, "t14_c", "s2").await;
    decided(p, "t14_c", id, None).await;
    let key = format!("change/{id}");
    assert_eq!(
        differing(p, instance("t14_b"), instance("t14_c")).await,
        [key.as_str()]
    );
    p.start(start("t14_d", binary, Some("s2"))).await.unwrap();
    waits_again(p, "t14_d", "s2", id).await;
    decided(p, "t14_d", id, None).await;
    assert!(differing(p, instance("t14_c"), instance("t14_d"))
        .await
        .is_empty());
    let fresh = propose(p, "t14_d").await;
    decided(p, "t14_d", &fresh, Some("no")).await;
}

async fn stop_all(w: &World, p: &Sculptured, ids: &[&str]) {
    // gc leaves the instances this value started alone
    p.gc(GcParams {
        keep: vec!["s*".into()],
    })
    .await
    .unwrap();
    assert_eq!(changes(p, ids[0]).await.as_array().unwrap().len(), 1);
    for i in ids {
        p.stop(IdParams {
            instance: (*i).into(),
        })
        .await
        .unwrap();
    }
    p.stop(IdParams {
        instance: "t14_a".into(),
    })
    .await
    .unwrap(); // stopped already: a no-op
    assert!(w.running().is_empty(), "servers left: {}", w.running());
    for s in ["s1", "s2"] {
        p.drop_save(SaveId { save: s.into() }).await.unwrap();
    }
    let left: Vec<_> = fs::read_dir(w.saves()).unwrap().flatten().collect();
    assert!(left.iter().all(|e| e.file_name() == ".live"), "{left:?}");
}

#[tokio::test]
#[ignore]
async fn live_sculptured_roundtrip() {
    let Some(binary) = binary() else {
        eprintln!("no sculptured program: live_sculptured_roundtrip skipped");
        return;
    };
    let w = World::new();
    let p = w.part();
    p.start(start("t14_a", &binary, None)).await.unwrap();
    let id = propose(&p, "t14_a").await;
    save_as(&w, "t14_a", "s1").await;
    decided(&p, "t14_a", &id, None).await;
    refusals(&p, "t14_a", &id).await;
    let key = format!("change/{id}");
    assert_eq!(
        differing(&p, instance("t14_a"), save("s1")).await,
        [key.as_str()]
    );
    restores_decide(&w, &p, &binary, &id).await;
    stop_all(&w, &p, &["t14_a", "t14_b", "t14_c", "t14_d"]).await;
}
