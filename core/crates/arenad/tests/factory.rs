//! The factory benchmark's world, whole: ERPNext, the hub, the stand-in plant and Sculptured, walked by a
//! scripted team (no model) in which each agent does its own part and no agent decides a change it
//! proposed. The right path passes every named check; a wrong one fails the order.
//!
//! `live_`: it starts an ERPNext stack of its own, and needs the Sculptured program and the factory
//! example, which are not public yet, so it says why and passes without them. Everything it makes is
//! removed when it ends. Run: `cargo test -p arenad --test factory -- --ignored live_ --nocapture`.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    path::PathBuf,
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
    runner,
};
use packages::{arena, bin_dir, package_roots};
use parts::{Hub, Registry};
use pg::{pg_url, Cleanup};
use serde_json::{json, Map, Value};

const MACHINE: &str = "PR-A1";
const PART: &str = "SP-SPN-800";
const VENDOR: &str = "Midstate Bearing & Spindle";
const CUSTOMERS: [(&str, &str); 3] = [
    ("Northfield Motors", "A-1"),
    ("Arcadia Trucks", "A-2"),
    ("Velo EV", "A-3"),
];
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

    /// `name`'s answer once `ready` says it holds what is waited for; the clock runs meanwhile.
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
            tokio::time::sleep(Duration::from_millis(100)).await;
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

    async fn propose_notices(&self) -> Result<(), String> {
        for (customer, order) in CUSTOMERS {
            let notice = json!({"customer": customer, "order": order, "new_date": NEW_DATE,
                                "message": NOTICE});
            self.propose(order, "customer_notice", notice).await?;
        }
        Ok(())
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
        for (customer, order) in CUSTOMERS {
            let id = self.approve(order).await?;
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
        self.notify().await
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

/// The ERPNext stack and server, the saves and the sites of this test; removed on drop.
struct Stack {
    id: String,
    saves: PathBuf,
}

impl Stack {
    fn new() -> Stack {
        let id = format!("t13{}", std::process::id());
        let saves = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("factory-{id}"));
        std::fs::create_dir_all(&saves).unwrap();
        Stack { id, saves }
    }

    fn env(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("ARENA_PG_URL".to_string(), pg_url()),
            ("PATH".to_string(), path_with_sculptured()),
        ];
        let port = |n: u16| (n + (std::process::id() % 1000) as u16).to_string();
        for (k, v) in [
            (
                "ARENA_ERPNEXT_DB_CONTAINER",
                format!("arena-erpnext-db-{}", self.id),
            ),
            (
                "ARENA_ERPNEXT_DB_VOLUME",
                format!("arena-erpnext-db-{}", self.id),
            ),
            ("ARENA_ERPNEXT_DB_PORT", port(23000)),
            (
                "ARENA_ERPNEXT_PROJECT",
                format!("arena_erpnext_{}", self.id),
            ),
            ("ARENA_ERPNEXT_PORT", port(24000)),
        ] {
            out.push((k.to_string(), v));
        }
        out
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let db = format!("arena-erpnext-db-{}", self.id);
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", &db])
            .output();
        let compose = arena().join("integrations/erpnext/compose.yml");
        let project = format!("arena_erpnext_{}", self.id);
        let mut down = Command::new("docker");
        down.args(["compose", "-p", &project, "-f"]).arg(&compose);
        down.env("ARENA_ERPNEXT_SITES", self.saves.join("erpnext/.sites"));
        let _ = down.args(["down", "-v"]).output();
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &db])
            .output();
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

/// `PATH` with the Sculptured program to use first: `ARENA_SCULPTURED_BINARY`, else a release build
/// next to the arena checkout, else the one already on it.
fn path_with_sculptured() -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let local = arena().join("../sculptured/target/release/sculptured");
    let set = std::env::var_os("ARENA_SCULPTURED_BINARY").map(PathBuf::from);
    let binary = set.or_else(|| local.exists().then_some(local));
    match binary.as_deref().and_then(|b| b.parent()) {
        Some(dir) => format!("{}:{path}", dir.display()),
        None => path,
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
        ])
        .current_dir(arena())
        .status()
        .unwrap();
    assert!(st.success(), "building the package binaries failed");
}

fn scripted_team(hubteam: parts::proto::BenchTeam) -> Team {
    let agent = |a: parts::proto::TeamRole| AgentSpec {
        id: a.id,
        name: a.name,
        kind: AgentKind::Open,
        model: Some("scripted".into()),
        prompt: a.prompt,
        provider: None,
        read_only: a.read_only,
    };
    Team {
        id: hubteam.id,
        name: hubteam.name,
        agents: hubteam.agents.into_iter().map(agent).collect(),
    }
}

/// Runs the team on one path to its end; the status and the checks that failed.
async fn walk(state: &Arc<AppState>, team: Team) -> (Branch, CheckPayload) {
    let (_run, id) = runner::start_run(state, "press-a1-bearing", team)
        .await
        .unwrap();
    for _ in 0..3000 {
        let b = state.store.branch(&id).await.unwrap();
        if b.status != BranchStatus::Running {
            let evs = state.store.events(&id).await.unwrap();
            let check = evs.iter().find(|e| e.kind == EventKind::Check);
            let payload = check
                .unwrap_or_else(|| panic!("no check: {b:?}"))
                .payload
                .clone();
            return (b, serde_json::from_value(payload).unwrap());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the branch did not finish");
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
    let stack = Stack::new();
    let hub = Hub::new(
        Registry::scan(&package_roots()).unwrap(),
        bin_dir(),
        stack.saves.clone(),
    );
    let hub = Arc::new(hub.with_env(stack.env()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(assert_paths(&hub));
}
