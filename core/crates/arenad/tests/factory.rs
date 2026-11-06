//! The factory benchmark's world, whole: ERPNext, Twenty, the hub, the stand-in plant and Sculptured, walked by a
//! scripted team (no model) in which each agent does its own part and no agent decides a change it
//! proposed. The right path passes every named check; a wrong one fails the order.
//!
//! `live_`: it starts an ERPNext stack and a Twenty server of its own, and needs the Sculptured program and the factory
//! example, which are not public yet, so it says why and passes without them. Everything it makes is
//! removed when it ends. Run: `cargo test -p arenad --test factory -- --ignored live_ --nocapture`.
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
    time::Duration,
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

const TASK: &str = "press-a1-bearing";
const MACHINE: &str = "PR-A1";
const PART: &str = "SP-SPN-800";
const VENDOR: &str = "Midstate Bearing & Spindle";
/// The press's line, as the ERP's workstation, and the last due date the outage affects (the
/// plant's first day, 2026-03-10, plus three days).
const LINE_WORKSTATION: &str = "Press Line A";
const AFFECTED_UNTIL: &str = "2026-03-13";
const FAILURE: &str = "Spindle bearing outer race: peak at BPFO 47.98 Hz";
const CARRIER: &str = "Interstate Hotshot";
const NEW_DATE: &str = "2026-03-14";
const NOTICE: &str = "PR-A1 is down; here is the new date.";

/// The quote in an answer whose `field` is `name`, once it has come (who has not answered yet is
/// in the answer too).
fn quote_of<'a>(answer: &'a Value, field: &str, name: &str) -> Option<&'a Value> {
    let quotes = answer["quotes"].as_array()?;
    quotes.iter().find(|q| q[field] == name)
}

/// One agent's calls, on one branch, and the notebook the scripted team passes ids in (a real team
/// would put them in its handoff notes).
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

    /// `name`'s answer once `ready` says it holds what is waited for, asked 600 times at most,
    /// every 2 s of arena time (every 100 ms at the test's speed of 20); the clock runs meanwhile.
    async fn wait(
        &self,
        name: &str,
        args: Value,
        ready: impl Fn(&Value) -> bool,
    ) -> Result<Value, String> {
        for _ in 0..600 {
            let v = self.call(name, args.clone()).await?;
            if ready(&v) {
                return Ok(v);
            }
            tokio::time::sleep(poll_interval()).await;
        }
        Err(format!("{name} never had what was waited for"))
    }

    /// Proposes a change, noted under `key`, for another agent to decide.
    async fn propose(&self, key: &str, action: &str, payload: Value) -> Result<(), String> {
        let proposal = json!({"action": action, "summary": action, "payload": payload});
        let change = self.call("propose_change", proposal).await?;
        let id = change["id"].as_str().ok_or("the change has no id")?;
        self.put(key, json!(id));
        Ok(())
    }

    /// Approves the change noted under `key`; its id.
    async fn approve(&self, key: &str) -> Result<Value, String> {
        let id = self.got(key)?;
        self.call("decide_change", json!({"id": id, "approved": true}))
            .await?;
        Ok(id)
    }

    async fn handoff(&self) -> Result<(), String> {
        let note = format!("{} is done; the ids are in the notes", self.agent);
        self.call("handoff", json!({"note": note})).await?;
        Ok(())
    }
}

/// 2 s of arena time at `ARENA_CLOCK_SPEED`, and never under 100 ms.
fn poll_interval() -> Duration {
    let speed = std::env::var("ARENA_CLOCK_SPEED").ok();
    let speed: f64 = speed.and_then(|s| s.parse().ok()).unwrap_or(20.0);
    Duration::from_secs_f64((2.0 / speed).max(0.1))
}

/// Maintenance: the trip, the evidence, and a maintenance order for the failure it diagnosed.
impl Calls<'_> {
    async fn diagnose(&self, failure: &str) -> Result<(), String> {
        let tripped = |v: &Value| v.to_string().contains("\"kind\":\"alarm\"");
        let args = json!({"since": 0, "limit": 500});
        self.wait("recent_events", args, tripped).await?;
        self.call("vibration_spectrum", json!({"machine": MACHINE}))
            .await?;
        self.call("list_docs", json!({"doctype": "Note"})).await?;
        let order = json!({"machine": MACHINE, "failure": failure, "technician": "T-1",
                           "planned_hours": 5});
        self.propose("maintenance_order", "maintenance_order", order)
            .await
    }
}

/// Procurement: quotes for the part and an order to `vendor` for `item`, then freight for it.
impl Calls<'_> {
    async fn source(&self, item: &str, vendor: &str) -> Result<(), String> {
        let rfq = self
            .call("request_quotes", json!({"part": PART, "qty": 1}))
            .await?;
        let rfq = rfq["rfq"].as_str().ok_or("no rfq")?.to_string();
        let has_vendor = |v: &Value| quote_of(v, "supplier", vendor).is_some();
        let quotes = self
            .wait("get_quotes", json!({"rfq": rfq}), has_vendor)
            .await?;
        let price = quote_of(&quotes, "supplier", vendor).ok_or("no quote")?["unit_price"].clone();
        let order = json!({"supplier": vendor, "item": item, "qty": 1, "unit_price": price,
                           "total": price, "need_by": "2026-03-12"});
        self.put(
            "order",
            json!({"rfq": rfq, "item": item, "vendor": vendor, "price": price}),
        );
        self.propose("purchase_order", "purchase_order", order)
            .await
    }

    /// Freight quotes, and the hotshot proposed.
    async fn freight(&self) -> Result<(), String> {
        let ask = json!({"origin": "Midstate Bearing & Spindle, Fort Wayne IN", "weight_kg": 310});
        let freight = self.call("freight_quotes", ask).await?;
        let request = freight["request"].as_str().ok_or("no request")?.to_string();
        let hotshot = |v: &Value| quote_of(v, "carrier", CARRIER).is_some();
        let args = json!({"request": request});
        let quotes = self.wait("get_freight_quotes", args, hotshot).await?;
        let quote = quote_of(&quotes, "carrier", CARRIER).ok_or("no quote")?;
        let (id, amount) = (quote["quote_id"].clone(), quote["amount"].clone());
        self.put("freight", json!({"quote_id": id, "amount": amount}));
        let change = json!({"carrier": CARRIER, "quote_id": id, "amount": amount,
                            "need_by": "2026-03-12"});
        self.propose("freight_booking", "freight_booking", change)
            .await
    }
}

/// Finance: decides Procurement's changes, awards the order and records it in the ERP.
impl Calls<'_> {
    async fn award(&self) -> Result<(), String> {
        let po = self.approve("purchase_order").await?;
        let order = self.got("order")?;
        let award = json!({"rfq": order["rfq"], "supplier": order["vendor"], "po": po});
        self.call("award_order", award).await?;
        self.record_order(&order).await?;
        if self.got("freight_booking").is_ok() {
            self.approve("freight_booking").await?;
        }
        Ok(())
    }

    /// The order in the ERP, submitted.
    async fn record_order(&self, order: &Value) -> Result<(), String> {
        let line = json!({"item_code": order["item"], "qty": 1, "rate": order["price"],
                          "schedule_date": "2026-12-01"});
        let doc = json!({"doctype": "Purchase Order", "supplier": order["vendor"],
                         "schedule_date": "2026-12-01", "items": [line]});
        let po = self.call("insert_doc", json!({"doc": doc})).await?;
        let name = po["name"].as_str().ok_or("no order name")?;
        let submit = json!({"doctype": "Purchase Order", "name": name});
        self.call("submit_doc", submit).await?;
        Ok(())
    }
}

/// Production planning: books the approved freight, schedules the repair, proposes the notices.
impl Calls<'_> {
    async fn book(&self) -> Result<(), String> {
        let (proposal, freight) = (self.got("freight_booking")?, self.got("freight")?);
        let booking = json!({"proposal_id": proposal, "quote_id": freight["quote_id"],
                             "carrier": CARRIER, "amount": freight["amount"]});
        self.call("book_freight", booking).await?;
        Ok(())
    }

    /// A ToDo for the repair, once the plant knows when the part arrives.
    async fn schedule(&self) -> Result<(), String> {
        let booked = |v: &Value| {
            v.to_string().contains(PART) && v["shipments"].as_array().is_some_and(|s| !s.is_empty())
        };
        let state = self.wait("plant_state", json!({}), booked).await?;
        let arrives = state["shipments"][0]["arrives"]
            .as_str()
            .ok_or("no arrival")?;
        let doc = json!({"doctype": "ToDo", "date": &arrives[..10],
                         "description": format!("Repair {MACHINE}: fit {PART}")});
        self.call("insert_doc", json!({"doc": doc})).await?;
        Ok(())
    }

    /// `field` of the documents of `doctype` matching `filters`, all of them.
    async fn listed(
        &self,
        doctype: &str,
        filters: Value,
        field: &str,
    ) -> Result<Vec<Value>, String> {
        let args = json!({"doctype": doctype, "filters": filters, "fields": [field], "limit": 0});
        let rows = self.call("list_docs", args).await?;
        Ok(rows.as_array().ok_or("no rows")?.clone())
    }

    /// The orders the outage affects, `[order id, customer]`: job cards on the line's workstation,
    /// their work orders due by the end of the window, and those orders' sales orders.
    async fn affected_orders(&self) -> Result<Vec<(String, String)>, String> {
        let cards = self.listed(
            "Job Card",
            json!({"workstation": LINE_WORKSTATION}),
            "work_order",
        );
        let works: Vec<Value> = cards
            .await?
            .iter()
            .map(|c| c["work_order"].clone())
            .collect();
        let due = json!([
            ["name", "in", works],
            ["expected_delivery_date", "<=", AFFECTED_UNTIL]
        ]);
        let sales = self.listed("Work Order", due, "sales_order").await?;
        let sales: Vec<Value> = sales.iter().map(|w| w["sales_order"].clone()).collect();
        let args = json!({"doctype": "Sales Order", "filters": [["name", "in", sales]],
                          "fields": ["po_no", "customer"], "limit": 0});
        let rows = self.call("list_docs", args).await?;
        let text = |r: &Value, k: &str| r[k].as_str().unwrap_or_default().to_string();
        let rows = rows.as_array().ok_or("no sales orders")?;
        Ok(rows
            .iter()
            .map(|r| (text(r, "po_no"), text(r, "customer")))
            .collect())
    }

    async fn propose_notices(&self) -> Result<(), String> {
        let orders = self.affected_orders().await?;
        self.put("orders", json!(orders));
        for (order, customer) in &orders {
            let notice = json!({"customer": customer, "order": order, "new_date": NEW_DATE,
                                "message": NOTICE});
            self.propose(order, "customer_notice", notice).await?;
        }
        Ok(())
    }

    /// The affected orders Planning found, from the notebook.
    fn orders(&self) -> Result<Vec<(String, String)>, String> {
        serde_json::from_value(self.got("orders")?).map_err(|e| e.to_string())
    }
}

/// The Incident lead: decides Maintenance's order and Planning's notices, and acts on them.
impl Calls<'_> {
    async fn dispatch(&self) -> Result<(), String> {
        let id = self.approve("maintenance_order").await?;
        let go =
            json!({"proposal_id": id, "technician": "T-1", "machine": MACHINE, "task": "inspect"});
        self.call("dispatch_technician", go).await?;
        Ok(())
    }

    async fn notify(&self) -> Result<(), String> {
        for (order, customer) in self.orders()? {
            let id = self.approve(&order).await?;
            let send = json!({"proposal_id": id, "customer": customer, "order": order,
                              "new_date": NEW_DATE, "message": NOTICE});
            self.call("notify_customer", send).await?;
        }
        Ok(())
    }
}

/// The right path: each agent does its own part of the incident.
impl Calls<'_> {
    async fn gold(&self) -> Result<(), String> {
        match self.agent {
            "maintenance" => self.diagnose(FAILURE).await?,
            "procurement" => {
                self.source(PART, VENDOR).await?;
                self.freight().await?;
            }
            "finance" => self.award().await?,
            "planning" => {
                self.book().await?;
                self.schedule().await?;
                self.propose_notices().await?;
            }
            "lead" => return self.lead_gold().await,
            other => return Err(format!("no script for {other}")),
        }
        self.handoff().await
    }

    async fn lead_gold(&self) -> Result<(), String> {
        self.dispatch().await?;
        self.notify().await?;
        self.record_in_crm().await
    }

    /// A note on each customer's account, and each noticed order's opportunity moved to the new
    /// date.
    async fn record_in_crm(&self) -> Result<(), String> {
        let orders = self.orders()?;
        let mut customers: Vec<&String> = orders.iter().map(|(_, c)| c).collect();
        customers.sort();
        customers.dedup();
        for customer in customers {
            let found = self.call("crm_find_account", json!({"name": customer, "limit": 1}));
            let note = json!({"account_id": found.await?["accounts"][0]["id"],
                              "title": format!("{MACHINE} down"), "body": NOTICE});
            self.call("crm_log_note", note).await?;
        }
        for (order, _) in &orders {
            let found = self.call(
                "crm_find_opportunities",
                json!({"name": format!("{order} ")}),
            );
            let deal = found.await?["opportunities"][0]["id"].clone();
            let moved = json!({"id": deal, "close_date": NEW_DATE});
            self.call("crm_update_opportunity", moved).await?;
        }
        Ok(())
    }

    /// The wrong diagnosis and the wrong part; a booking with no change behind it, which the gate
    /// must refuse.
    async fn wrong(&self) -> Result<(), String> {
        match self.agent {
            "maintenance" => self.diagnose("Worn flywheel brake").await?,
            "procurement" => self.source("SP-SPN-600", "Orion Precision Parts").await?,
            "finance" => self.award().await?,
            "planning" => self.refused_booking().await?,
            "lead" => return self.dispatch().await,
            other => return Err(format!("no script for {other}")),
        }
        self.handoff().await
    }

    async fn refused_booking(&self) -> Result<(), String> {
        let booking = json!({"proposal_id": "no-such-change", "quote_id": "FRQ-001-1",
                             "carrier": "Lakeshore Freight", "amount": 420});
        let refused = self.call("book_freight", booking).await.unwrap_err();
        assert!(refused.contains("book_freight refused"), "{refused}");
        Ok(())
    }
}

/// Every agent does its own part of `gold` or the wrong path, and all but the lead hand off.
struct Scripted {
    state: std::sync::Weak<AppState>,
    gold: bool,
    notes: Mutex<Map<String, Value>>,
}

impl Scripted {
    async fn act(&self, run: &AgentRun) -> Result<(), String> {
        let state = self.state.upgrade().ok_or("no app")?;
        let exec = ToolExec::new(state.store.clone());
        let calls = Calls {
            exec: &exec,
            branch: &run.branch,
            agent: run.spec.id.as_str(),
            notes: &self.notes,
        };
        match self.gold {
            true => calls.gold().await,
            false => calls.wrong().await,
        }
    }
}

impl Driver for Scripted {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let done = self.act(&run).await;
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

/// Why this machine can't run the world, if it can't.
fn missing() -> Option<String> {
    let example = arena().join("../Sculptured-Examples/factory/data/ridgeline.json");
    if !example.exists() {
        return Some(format!("no factory example at {}", example.display()));
    }
    let mut version = Command::new("sculptured");
    let found = version
        .arg("--version")
        .env("PATH", path_with_sculptured())
        .output();
    found.is_err().then(|| "no sculptured program".to_string())
}

fn build() {
    let st = Command::new("cargo")
        .args([
            "build",
            "-q",
            "-p",
            "arena-factory",
            "-p",
            "arena-erpnext",
            "-p",
            "arena-sculptured",
            "-p",
            "arena-twenty-crm",
        ])
        .current_dir(arena())
        .status()
        .unwrap();
    assert!(st.success(), "building the package binaries failed");
}

/// Runs the team on one path to its end (10 minutes at most); the status and the checks.
async fn walk(state: &Arc<AppState>, team: Team) -> (Branch, CheckPayload) {
    let (b, check, _) = walk_for(state, TASK, team, 3000).await;
    (b, check)
}

async fn app(hub: &Arc<Hub>, tag: &str, gold: bool) -> (Arc<AppState>, Cleanup) {
    let (store, clean) = app_support::meta_store("factory", tag).await;
    let state = AppState::new(store, hub.clone(), Config::new()).with_benchmark("factory");
    let state = Arc::new(state);
    let driver: Arc<dyn Driver> = Arc::new(Scripted {
        state: Arc::downgrade(&state),
        gold,
        notes: Mutex::default(),
    });
    assert!(state.install_drivers([(AgentKind::Open, driver)].into_iter().collect()));
    (state, clean)
}

async fn assert_paths(hub: &Arc<Hub>) {
    let bench = hub.bench_describe("factory").await.unwrap();
    let team = bench.team.clone().unwrap();
    let (state, _clean) = app(hub, "gold", true).await;
    let (b, check) = walk(&state, scripted_team(team.clone())).await;
    assert_eq!(check.differing_rows, Some(vec![]), "{check:?}");
    assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
    let (state, _clean) = app(hub, "wrong", false).await;
    let (b, check) = walk(&state, scripted_team(team)).await;
    assert_eq!(b.status, BranchStatus::Failed, "{:?}", b.error);
    let failed = check.differing_rows.unwrap();
    for name in [
        "diagnosis_outer_race",
        "correct_part_ordered_and_approved",
        "shipment_reached_plant",
    ] {
        assert!(
            failed.iter().any(|f| f == name),
            "{name} passed: {failed:?}"
        );
    }
}

#[test]
#[ignore]
fn live_factory_right_path_passes_and_wrong_path_fails() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    build();
    // SAFETY: set before the runtime or any thread that reads the environment starts.
    unsafe { std::env::set_var("ARENA_CLOCK_SPEED", "20") };
    let stack = Stack::new("t13");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(assert_paths(&hub));
}

/// The gold path only, then its checks and metrics (measured time, arena's overhead, the
/// estimate in production) printed as JSON. For a run whose parts are not the defaults, set by
/// the environment: `ARENA_RUN_FACTORY` (e.g. `--backend isaac --plant-config ...`),
/// `ARENA_RUN_ISAAC_SIM` (the plant on another machine) and `ARENA_CLOCK_SPEED` (default 20). It
/// waits up to an hour, since Isaac Sim starts and steps far slower than the stand-in.
#[test]
#[ignore]
fn live_factory_gold_path_report() {
    if let Some(why) = missing() {
        eprintln!("skipped: {why}");
        return;
    }
    build();
    if std::env::var_os("ARENA_CLOCK_SPEED").is_none() {
        // SAFETY: set before the runtime or any thread that reads the environment starts.
        unsafe { std::env::set_var("ARENA_CLOCK_SPEED", "20") };
    }
    let stack = Stack::new("t13");
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(report_gold(&hub));
}

async fn report_gold(hub: &Arc<Hub>) {
    let bench = hub.bench_describe("factory").await.unwrap();
    let (state, _clean) = app(hub, "report", true).await;
    let team = scripted_team(bench.team.unwrap());
    let (b, check, events) = walk_for(&state, TASK, team, 18_000).await;
    let m = arenad::metrics::metrics(&b, &events, arenad::prices::table(), chrono::Utc::now());
    let report = json!({"status": b.status, "error": b.error, "check": check, "metrics": m});
    println!("REPORT {}", serde_json::to_string_pretty(&report).unwrap());
    assert_eq!(check.differing_rows, Some(vec![]), "{check:?}");
    assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
}
