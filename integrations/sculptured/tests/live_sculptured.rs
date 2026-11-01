//! Runs real `sculptured serve` instances: `cargo test -p arena-sculptured -- --ignored live_`.
//! Needs the sculptured program (default `~/MLSys/Sculptured/sculptured/target/release/sculptured`,
//! or env `ARENA_SCULPTURED_BINARY`); without it the test says so and passes. Every server and
//! folder the test makes is removed afterwards, even on failure.
use arena_part::helpers::process_group::{kill_matching, matching};
use arena_part::proto::*;
use arena_part::Part;
use arena_sculptured::Sculptured;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// The test's folder; dropping it kills every process started under it and removes it.
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

    fn running(&self) -> Vec<u32> {
        matching(&self.dir.to_string_lossy())
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = kill_matching(&self.dir.to_string_lossy(), Duration::from_secs(10));
        let _ = fs::remove_dir_all(&self.dir);
        // the servers' scratch folders (`arena-sc-<pid>-<n>`, outside the test's folder)
        let mine = format!("arena-sc-{}-", std::process::id());
        let tmp = fs::read_dir(std::env::temp_dir())
            .into_iter()
            .flatten()
            .flatten();
        for e in tmp.filter(|e| e.file_name().to_string_lossy().starts_with(&mine)) {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

fn binary() -> Option<PathBuf> {
    let b = std::env::var_os("ARENA_SCULPTURED_BINARY")
        .map(PathBuf::from)
        .or_else(|| {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            Some(home.join("MLSys/Sculptured/sculptured/target/release/sculptured"))
        })?;
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

fn instance(i: &str) -> Target {
    Target::Instance { instance: i.into() }
}

fn save(s: &str) -> Target {
    Target::Save { save: s.into() }
}

async fn differing(p: &Sculptured, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
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
    let listed = tool(p, inst, "list_changes", json!({}))
        .await
        .result
        .unwrap();
    assert_eq!(listed["changes"][0], change);
    id
}

/// A refusal is the tool's error, and it changes nothing.
fn refused(out: &ToolOutput, words: &str) {
    let e = out.error.as_deref().unwrap_or_default();
    assert!(e.contains(words), "{out:?}");
    assert!(out.result.is_none() && out.reads.is_empty() && out.writes.is_empty());
}

/// `decide_change` with `args` on `inst`.
async fn decide(p: &Sculptured, inst: &str, args: Value) -> ToolOutput {
    tool(p, inst, "decide_change", args).await
}

async fn decide_and_refusals(p: &Sculptured, inst: &str, id: &str) {
    let out = decide(p, inst, json!({"id": id, "approved": true})).await;
    let change = out.result.expect("decide_change gives the change");
    assert_eq!(change["decision"]["approved"], true, "{change}");
    assert_eq!(change["decision"]["by"], "person");
    assert_eq!(out.writes, [format!("change/{id}")]);
    let again = decide(p, inst, json!({"id": id, "approved": false})).await;
    refused(&again, "already decided: approved by person");
    let nope = decide(p, inst, json!({"id": "chg-nope", "approved": true})).await;
    refused(&nope, "no change chg-nope");
    refused(&decide(p, inst, json!({"id": id})).await, "approved");
    let bad = tool(
        p,
        inst,
        "propose_change",
        json!({"action": "x", "summary": "s"}),
    )
    .await;
    refused(&bad, "payload must be an object");
    let unknown = CallToolParams {
        instance: inst.into(),
        name: "nope".into(),
        args: json!({}),
        clock_ms: 0,
    };
    assert_eq!(p.call_tool(unknown).await.unwrap_err().code, -32602);
}

/// `b` started from `s1` has the change undecided again, and is a working instance.
async fn check_restored(p: &Sculptured, b: &str, id: &str) {
    let key = format!("change/{id}");
    let docs = p
        .inspect(InspectParams {
            target: instance(b),
            keys: vec![key.clone(), "change/nope".into()],
        })
        .await
        .unwrap()
        .docs;
    assert_eq!(docs[&key]["decision"], Value::Null, "{docs:?}");
    assert_eq!(docs["change/nope"], Value::Null);
    assert!(differing(p, instance(b), save("s1")).await.is_empty());
    // its question went with the server that saved it: Sculptured can't decide it any more
    let out = decide(p, b, json!({"id": id, "approved": true})).await;
    refused(&out, "can no longer decide");
    let fresh = propose(p, b).await;
    let no = json!({"id": fresh, "approved": false, "note": "too dear"});
    let decided = decide(p, b, no).await.result.unwrap();
    assert_eq!(decided["decision"]["approved"], false);
    assert_eq!(decided["decision"]["note"], "too dear");
}

async fn stop_all(w: &World, p: &Sculptured, ids: [&str; 2]) {
    for i in ids {
        p.stop(IdParams { instance: i.into() }).await.unwrap();
    }
    assert!(w.running().is_empty(), "servers left: {:?}", w.running());
    p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
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
    let (a, b) = ("t14_a", "t14_b");
    let w = World::new();
    let p = w.part();
    p.start(start(a, &binary, None)).await.unwrap();
    let id = propose(&p, a).await;
    let params = SaveParams {
        instance: a.into(),
        save: "s1".into(),
        dir: w.saves().join("s1").to_string_lossy().into(),
    };
    let saved = w.part().save(params).await.unwrap();
    assert!(saved.exact && saved.bytes > 0, "{saved:?}");
    decide_and_refusals(&p, a, &id).await;
    assert_eq!(
        differing(&p, instance(a), save("s1")).await,
        [format!("change/{id}")]
    );
    p.start(start(b, &binary, Some("s1"))).await.unwrap();
    check_restored(&p, b, &id).await;
    stop_all(&w, &w.part(), [a, b]).await;
}
