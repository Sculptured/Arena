//! The pharma benchmark's world, whole: ERPNext, Twenty, the simulated LIMS and carriers and
//! Sculptured's approvals, walked by the coordinated team scripted (no model): each role does its
//! own part, and every change is decided by the simulated QA head or supply chain director through
//! the world's links. The right path passes every check of the base and of the rejected
//! reallocation; a wrong one (release the failing batch without approval and ship it) fails the
//! hold and safety checks.
//!
//! `live_`: it starts an ERPNext stack and a Twenty server of its own and needs the Sculptured
//! program; everything it makes is removed when it ends. Run:
//! `cargo test -p arenad --test pharma -- --ignored live_ --nocapture`.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/live_world.rs"]
mod live_world;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    pin::Pin,
    process::Command,
    sync::{Arc, Mutex},
};

use arenad::{
    agents::{AgentResult, AgentRun, Driver},
    app::{AppState, Config},
    exec::{BranchCtx, ToolExec},
    model::*,
};
use live_world::{path_with_sculptured, scripted_team, walk_for, Stack};
use packages::{arena, bin_dir, package_roots};
use parts::{Hub, Registry};
use pg::Cleanup;
use serde_json::{json, Map, Value};

const HELD: &str = "CVX-2611";
const SAMPLE: &str = "EM-26-11873";
const NEXT_LOT: &str = "CVX-2608";
const RESERVED: &str = "CVX-2598";

/// The gold allocation: order, lot, where the lot is, and the carrier.
const PLAN: [(&str, &str, &str, &str); 8] = [
    ("PO-SBMC-4471", "CVX-2604", "US", "Tundra Medical Courier"),
    ("PO-HVH-20931", "CVX-2589", "US", "Tundra Medical Courier"),
    ("PO-MSD-88102", NEXT_LOT, "US", "Tundra Medical Courier"),
    ("PO-LCH-3307", "CVX-2604", "US", "Tundra Medical Courier"),
    ("PO-CHP-61540", NEXT_LOT, "US", "Tundra Medical Courier"),
    ("PO-PWD-7719", RESERVED, "EU", "Borealis Cold Chain"),
    ("PO-ARH-1186", "CVX-2589", "US", "Tundra Medical Courier"),
    ("PO-MSD-88147", RESERVED, "EU", "Borealis Cold Chain"),
];

/// One agent's calls on one branch, and the notebook the scripted team passes facts in (a real
/// team would put them in its handoff notes).
struct Calls<'a> {
    exec: &'a ToolExec,
    branch: &'a BranchCtx,
    agent: &'a str,
    notes: &'a Mutex<Map<String, Value>>,
}

impl Calls<'_> {
    async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        let (b, a) = (self.branch, self.agent);
        self.exec.run_outcome(b, a, name, args, None).await
    }

    fn put(&self, key: &str, value: Value) {
        self.notes.lock().unwrap().insert(key.into(), value);
    }

    fn got(&self, key: &str) -> Result<Value, String> {
        let notes = self.notes.lock().unwrap();
        notes
            .get(key)
            .cloned()
            .ok_or(format!("no {key} in the notes"))
    }

    async fn handoff(&self) -> Result<(), String> {
        let note = format!("{} is done; the facts are in the notes", self.agent);
        self.call("handoff", json!({"note": note})).await.map(drop)
    }

    /// Proposes a change and reads back the person's decision (the links decide at once).
    async fn propose(&self, action: &str, payload: Value) -> Result<Value, String> {
        let args = json!({"action": action, "summary": action, "payload": payload});
        let change = self.call("propose_change", args).await?;
        let id = change["id"].clone();
        let all = self.call("list_changes", json!({})).await?;
        let list = all["changes"].as_array().ok_or("no changes")?;
        let decided = list
            .iter()
            .find(|c| c["id"] == id)
            .ok_or("the change is not listed")?;
        Ok(decided.clone())
    }

    /// Proposes until a person approves (once more after a rejection); the change's id.
    async fn approved(&self, action: &str, payload: Value) -> Result<Value, String> {
        for _ in 0..2 {
            let c = self.propose(action, payload.clone()).await?;
            if c["decision"]["approved"] == true {
                return Ok(c["id"].clone());
            }
        }
        Err(format!("{action} {payload} was not approved"))
    }
}

/// The coordinator and the records analyst only read; their facts go in the notes.
impl Calls<'_> {
    async fn coordinate(&self) -> Result<(), String> {
        let hit = self.call("affected_orders", json!({"batch": HELD})).await?;
        let orders = hit["orders"].as_array().ok_or("no orders")?;
        if orders.len() != PLAN.len() {
            return Err(format!(
                "{} affected orders, not {}",
                orders.len(),
                PLAN.len()
            ));
        }
        self.put("orders", hit["orders"].clone());
        Ok(())
    }

    async fn records(&self) -> Result<(), String> {
        self.call("stock_report", json!({})).await?;
        let filters = json!([["planned_batch", "=", "CVX-2615"]]);
        let args = json!({"doctype": "Work Order", "filters": filters, "fields": ["name"]});
        let next = self.call("list_docs", args).await?;
        self.put("next_work_order", next[0]["name"].clone());
        Ok(())
    }
}

/// Quality: the deviation, the hold, and the release of the tested lot once approved.
impl Calls<'_> {
    async fn quality(&self) -> Result<(), String> {
        self.call("lims_em_results", json!({"batch": HELD})).await?;
        let nc = json!({"doctype": "Non Conformance", "subject": format!("Grade A EM excursion, {HELD}"),
                        "procedure": "EM excursion investigation (SOP-QA-031)", "status": "Open",
                        "details": format!("{SAMPLE}: 3 CFU against an action limit of 1 during the fill of {HELD}.")});
        self.call("insert_doc", json!({"doc": nc})).await?;
        self.set_status(HELD, "On Hold").await?;
        self.approved("batch_hold", json!({"batch": HELD})).await?;
        self.call("lims_batch_results", json!({"batch": NEXT_LOT}))
            .await?;
        self.approved("batch_release", json!({"batch": NEXT_LOT}))
            .await?;
        self.set_status(NEXT_LOT, "Released").await
    }

    async fn set_status(&self, batch: &str, status: &str) -> Result<(), String> {
        let args = json!({"doctype": "Batch", "name": batch, "fields": {"qa_status": status}});
        self.call("update_doc", args).await.map(drop)
    }

    /// The wrong path's quality: the failing batch released, no deviation, no approval.
    async fn quality_wrong(&self) -> Result<(), String> {
        self.call("lims_get_document", json!({"id": "SOP-QA-031"}))
            .await?;
        self.set_status(HELD, "Released").await
    }
}

/// Supply: the reserved EU stock reallocated (proposed again if the first is rejected).
impl Calls<'_> {
    async fn supply(&self) -> Result<(), String> {
        let check = json!({"customer": "Pinecrest Wholesale Drug", "arrives": "2026-11-12"});
        self.call("eligible_lots", check).await?;
        let orders = ["PO-PWD-7719", "PO-MSD-88147"];
        let payload = json!({"batch": RESERVED, "qty": 1800, "orders": orders});
        self.approved("stock_reallocation", payload).await.map(drop)
    }

    async fn production(&self) -> Result<(), String> {
        self.call("production_schedule", json!({})).await?;
        let name = self.got("next_work_order")?;
        let fields = json!({"planned_start_date": "2026-11-16 06:00:00"});
        let args = json!({"doctype": "Work Order", "name": name, "fields": fields});
        self.call("update_doc", args).await.map(drop)
    }
}

/// Logistics: each order quoted, approved, booked and told, and the CRM updated.
impl Calls<'_> {
    /// The order's quantity and customer, from the coordinator's notes.
    fn order(&self, po: &str) -> Result<(Value, String), String> {
        let orders = self.got("orders")?;
        let list = orders.as_array().ok_or("no orders")?;
        let o = list
            .iter()
            .find(|o| o["po_no"] == po)
            .ok_or(format!("no order {po}"))?;
        Ok((
            o["total_qty"].clone(),
            o["customer"].as_str().unwrap_or_default().to_string(),
        ))
    }

    /// Books `batch` from `origin` with `carrier` for `po`; the arrival date.
    async fn ship(
        &self,
        po: &str,
        batch: &str,
        origin: &str,
        carrier: &str,
    ) -> Result<String, String> {
        let (qty, customer) = self.order(po)?;
        let qty = json!(qty.as_f64().ok_or("no qty")? as u64);
        let asked = json!({"origin": origin, "customer": customer, "qty": qty});
        let quotes = self.call("cold_chain_quotes", asked).await?;
        let list = quotes["quotes"].as_array().ok_or("no quotes")?;
        let q = list
            .iter()
            .find(|q| q["carrier"] == carrier)
            .ok_or("no quote")?;
        let shipment = json!({"carrier": carrier, "quote_id": q["quote_id"], "order": po, "batch": batch, "qty": qty});
        let id = self
            .approved("cold_chain_shipment", shipment.clone())
            .await?;
        let mut booking = shipment;
        booking["proposal_id"] = id;
        let booked = self.call("book_shipment", booking).await?;
        Ok(booked["arrives"].as_str().unwrap_or_default().to_string())
    }

    async fn tell(&self, po: &str, arrives: &str) -> Result<(), String> {
        let (_, customer) = self.order(po)?;
        let message = format!("Order {po} now ships from another lot and arrives on {arrives}.");
        let notice =
            json!({"customer": customer, "order": po, "new_date": arrives, "message": message});
        self.call("send_customer_notice", notice).await?;
        let found = self
            .call("crm_find_account", json!({"name": customer, "limit": 1}))
            .await?;
        let note = json!({"account_id": found["accounts"][0]["id"], "title": format!("{po}: new date"), "body": message});
        self.call("crm_log_note", note).await?;
        let deals = self
            .call("crm_find_opportunities", json!({"name": format!("{po} ")}))
            .await?;
        let moved = json!({"id": deals["opportunities"][0]["id"], "close_date": arrives});
        self.call("crm_update_opportunity", moved).await.map(drop)
    }

    async fn logistics(&self, plan: &[(&str, &str, &str, &str)]) -> Result<(), String> {
        for (po, batch, origin, carrier) in plan {
            let arrives = self.ship(po, batch, origin, carrier).await?;
            self.tell(po, &arrives).await?;
        }
        Ok(())
    }
}

/// Which path the scripted team walks.
#[derive(Clone, Copy, PartialEq)]
enum Path {
    Gold,
    Wrong,
}

impl Calls<'_> {
    async fn walk(&self, path: Path) -> Result<(), String> {
        match (self.agent, path) {
            ("coordinator", _) => self.coordinate().await?,
            ("records", _) => self.records().await?,
            ("quality", Path::Gold) => self.quality().await?,
            ("quality", Path::Wrong) => self.quality_wrong().await?,
            ("supply", Path::Gold) => self.supply().await?,
            ("production", Path::Gold) => self.production().await?,
            ("logistics", Path::Gold) => return self.logistics(&PLAN).await,
            ("logistics", Path::Wrong) => {
                return self.logistics(&[(PLAN[0].0, HELD, "US", PLAN[0].3)]).await
            }
            (_, Path::Wrong) => {}
            (other, _) => return Err(format!("no script for {other}")),
        }
        self.handoff().await
    }
}

struct Scripted {
    state: std::sync::Weak<AppState>,
    path: Path,
    notes: Mutex<Map<String, Value>>,
}

impl Driver for Scripted {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let done = match self.state.upgrade() {
                None => Err("no app".to_string()),
                Some(state) => {
                    let exec = ToolExec::new(state.store.clone());
                    let (branch, agent) = (&run.branch, run.spec.id.as_str());
                    let calls = Calls {
                        exec: &exec,
                        branch,
                        agent,
                        notes: &self.notes,
                    };
                    calls.walk(self.path).await
                }
            };
            AgentResult {
                ok: done.is_ok(),
                error: done.err(),
                tokens: json!({}),
                cost_usd: None,
                cost_source: None,
                seconds: 0.0,
                failure: None,
            }
        })
    }
}

async fn app(hub: &Arc<Hub>, tag: &str, path: Path) -> (Arc<AppState>, Cleanup) {
    let (store, clean) = app_support::meta_store("pharma", tag).await;
    let state = AppState::new(store, hub.clone(), Config::new()).with_benchmark("pharma");
    let state = Arc::new(state);
    let notes = Mutex::default();
    let driver: Arc<dyn Driver> = Arc::new(Scripted {
        state: Arc::downgrade(&state),
        path,
        notes,
    });
    assert!(state.install_drivers([(AgentKind::Open, driver)].into_iter().collect()));
    (state, clean)
}

/// Walks `path` on `task` (20 minutes at most); the branch and its check.
async fn walk(hub: &Arc<Hub>, task: &str, path: Path) -> (Branch, CheckPayload) {
    let bench = hub.bench_describe("pharma").await.unwrap();
    let (state, _clean) = app(hub, &task.replace('-', "_")[..12], path).await;
    let (b, check, _) = walk_for(&state, task, scripted_team(bench.team.unwrap()), 6000).await;
    println!("{task}: {:?} {:?} {check:?}", b.status, b.error);
    (b, check)
}

async fn assert_paths(hub: &Arc<Hub>) {
    for task in ["cvx-2611-em-excursion", "reallocation-rejected"] {
        let (b, check) = walk(hub, task, Path::Gold).await;
        assert_eq!(check.differing_rows, Some(vec![]), "{task}: {check:?}");
        assert_eq!(b.status, BranchStatus::Passed, "{task}: {:?}", b.error);
    }
    let (b, check) = walk(hub, "cvx-2611-em-excursion", Path::Wrong).await;
    assert_eq!(b.status, BranchStatus::Failed, "{:?}", b.error);
    let failed = check.differing_rows.unwrap();
    for name in [
        "deviation_opened",
        "batch_held",
        "no_held_or_unreleased_lot_shipped",
        "no_release_without_approval",
    ] {
        assert!(
            failed.iter().any(|f| f == name),
            "{name} passed: {failed:?}"
        );
    }
}

fn missing() -> Option<String> {
    let found = Command::new("sculptured")
        .arg("--version")
        .env("PATH", path_with_sculptured())
        .output();
    let uv = Command::new("uv").arg("--version").output();
    match (found.is_err(), uv.is_err()) {
        (true, _) => Some("no sculptured program".into()),
        (_, true) => Some("no uv (the simulated parts run with it)".into()),
        _ => None,
    }
}

fn build() {
    let packages = [
        "arena-pharma",
        "arena-erpnext",
        "arena-sculptured",
        "arena-twenty-crm",
    ];
    let mut args = vec!["build", "-q"];
    packages.iter().for_each(|p| args.extend(["-p", p]));
    let st = Command::new("cargo")
        .args(args)
        .current_dir(arena())
        .status()
        .unwrap();
    assert!(st.success(), "building the package binaries failed");
}

#[test]
#[ignore]
fn live_pharma_right_paths_pass_and_wrong_path_fails() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    build();
    let stack = Stack::new("pw");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(assert_paths(&hub));
}
