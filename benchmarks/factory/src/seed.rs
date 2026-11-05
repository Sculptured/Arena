//! The world's starting records: a fictional stamping company, from the factory example's data and
//! `data/company.json`, written into the ERP (`company.rs`) and the CRM (`crm.rs`) through their REST
//! APIs, with the notes the incident's team reads. The hub and the plant start from the example's own
//! data.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::company::{list, Company};
use crate::crm;
use crate::date::Day;
use crate::erp::{name_of, Erp};
use crate::facts::{machine, MACHINE};
use crate::scale;

/// Shaft speed in Hz and the defect-frequency rows of a press's service manual.
fn defect_table(drive: &Value) -> String {
    let hz = drive["shaft_rpm"].as_f64().unwrap_or(0.0) / 60.0;
    let row = |name: &str, key: &str| {
        let order = drive["defect_orders"][key].as_f64().unwrap_or(0.0);
        format!(
            "<tr><td>{name}</td><td>{order}</td><td>{:.2} Hz</td></tr>",
            order * hz
        )
    };
    let rows: Vec<String> = [
        ("Cage (FTF)", "ftf"),
        ("Outer race (BPFO)", "bpfo"),
        ("Inner race (BPFI)", "bpfi"),
        ("Rolling element (BSF)", "bsf"),
    ]
    .iter()
    .map(|(name, key)| row(name, key))
    .collect();
    format!("<table>{}</table>", rows.join(""))
}

/// The press's service manual: its bearing and which peak means which defect.
fn manual_note(data: &Value) -> Result<Value> {
    let press = machine(data)?;
    let kind = press["kind"].as_str().unwrap_or_default();
    let drive = &data["main_drives"][kind];
    if drive.is_null() {
        bail!("the data has no main drive for {kind}");
    }
    let content = format!(
        "<p>Main drive spindle bearing {}, shaft speed {} rpm. Replacement part {}.</p>\
         <p>Match the largest spectrum peaks against this table of multiples of shaft speed:</p>{}",
        drive["bearing"]["designation"].as_str().unwrap_or_default(),
        drive["shaft_rpm"],
        press["spindle_part"].as_str().unwrap_or_default(),
        defect_table(drive),
    );
    Ok(
        json!({"doctype": "Note", "title": format!("{MACHINE} service manual"), "public": 1,
              "content": content}),
    )
}

/// The press's service manual, the spares on the shelf, and the notice that all of this is made
/// up. Which orders a stopped line affects is for the team to find in the ERP's records.
pub fn notes(c: &Company) -> Result<Vec<Value>> {
    Ok(vec![
        manual_note(c.data)?,
        c.stock_note(),
        c.fictional_note(),
    ])
}

/// Writes each document, in order.
async fn insert_all(erp: &Erp, docs: &[Value]) -> Result<()> {
    for doc in docs {
        erp.insert(doc).await?;
    }
    Ok(())
}

/// The company, the calendar it books in, and the two as the site's default.
async fn company(erp: &Erp, c: &Company<'_>) -> Result<()> {
    for year in c.fiscal_years() {
        let filters = json!({"year": year["year"]});
        if erp.names("Fiscal Year", &filters).await?.is_empty() {
            erp.insert(&year).await?;
        }
    }
    erp.insert(&c.company_doc()).await?;
    let defaults = json!({"default_company": c.name()});
    erp.update("Global Defaults", "Global Defaults", &defaults)
        .await?;
    Ok(())
}

/// Each press as a submitted asset, with its plan and its past maintenance.
async fn presses(erp: &Erp, c: &Company<'_>) -> Result<()> {
    insert_all(erp, &c.team()).await?;
    insert_all(erp, &c.asset_masters()).await?;
    for m in list(c.data, "machines") {
        let asset = erp.submit_new(&c.asset(m)?).await?;
        let plan = erp.insert(&c.maintenance(m, name_of(&asset)?)?).await?;
        for log in c.maintenance_logs(m, &plan)? {
            erp.submit_new(&log).await?;
        }
    }
    Ok(())
}

/// How many orders the seed writes at once.
const PARALLEL: usize = 5;

/// Runs `job` on each of `items`, [`PARALLEL`] at a time; the first error, once all have ended.
pub async fn each_at_once<T, F, Fut>(items: Vec<T>, job: F) -> Result<()>
where
    T: Send + 'static,
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    let mut running = tokio::task::JoinSet::new();
    let mut first_error = None;
    for item in items {
        if running.len() >= PARALLEL {
            first_error = first_error.or(running.join_next().await.and_then(failed));
        }
        running.spawn(job(item));
    }
    while let Some(ended) = running.join_next().await {
        first_error = first_error.or(failed(ended));
    }
    first_error.map_or(Ok(()), Err)
}

fn failed(ended: Result<Result<()>, tokio::task::JoinError>) -> Option<anyhow::Error> {
    match ended {
        Ok(r) => r.err(),
        Err(e) => Some(anyhow!("a seed task ended: {e}")),
    }
}

/// Each order's sales order and its work order (whose `sales_order` the seed fills in).
fn order_docs(c: &Company<'_>, boms: &HashMap<String, String>) -> Result<Vec<(Value, Value)>> {
    let mut out = vec![];
    for o in list(c.data, "orders") {
        let bom = boms
            .get(&o["item"].to_string())
            .ok_or_else(|| anyhow!("no BOM for {}", o["item"]))?;
        out.push((c.sales_order(o)?, c.work_order(o, "", bom)?));
    }
    Ok(out)
}

/// The BOMs, submitted; each order's sales order and work order, submitted, with its job cards.
async fn orders(erp: &Arc<Erp>, c: &Company<'_>) -> Result<()> {
    let mut boms = HashMap::new();
    for bom in c.boms()? {
        let saved = erp.submit_new(&bom).await?;
        boms.insert(bom["item"].to_string(), name_of(&saved)?.to_string());
    }
    // Every work order draws the same coil from the same store, so their submits update one stock
    // row (`tabBin`): they go one at a time, while the sales orders go several at once.
    let one_at_a_time = Arc::new(tokio::sync::Mutex::new(()));
    each_at_once(order_docs(c, &boms)?, |(so, mut work)| {
        let (erp, turn) = (erp.clone(), one_at_a_time.clone());
        async move {
            let so = erp.submit_new(&so).await?;
            work["sales_order"] = json!(name_of(&so)?);
            let _turn = turn.lock().await;
            erp.submit_new(&work).await.map(drop)
        }
    })
    .await
}

/// Writes the company into the ERP through its REST API (`erp` is its `connect` info).
pub async fn seed_erp(erp: &Value, c: &Company<'_>) -> Result<()> {
    let erp = Arc::new(Erp::connect(erp)?);
    company(&erp, c).await?;
    insert_all(&erp, &c.sites()).await?;
    insert_all(&erp, &c.suppliers()).await?;
    for group in c.item_groups() {
        erp.insert_missing(&group).await?;
    }
    insert_all(&erp, &c.items()?).await?;
    insert_all(&erp, &c.prices()).await?;
    insert_all(&erp, &c.workstations()?).await?;
    insert_all(&erp, &c.customers()).await?;
    presses(&erp, c).await?;
    erp.submit_new(&c.opening_stock()?).await?;
    orders(&erp, c).await?;
    insert_all(&erp, &notes(c)?).await
}

/// What `data/company.json` adds to the factory example's data.
pub fn company_extra() -> Value {
    serde_json::from_str(include_str!("../data/company.json"))
        .expect("data/company.json is JSON (a test reads it)")
}

/// Loads the fictional company, at its scale (`scale.rs`), into the ERP and the CRM, dated from
/// the plant's first day.
pub async fn seed(erp: &Value, crm: &Value, data: &Value) -> Result<()> {
    let (data, extra) = scale::enlarge(data, &company_extra());
    let c = Company {
        data: &data,
        extra: &extra,
        today: Day::plant(),
    };
    seed_erp(erp, &c).await.context("the ERP")?;
    crm::seed(crm, &c).await.context("the CRM")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real file's shape, cut down: one line of the failing press, one line of another.
    fn data() -> Value {
        json!({
            "company": "Ridgeline Components", "currency": "USD",
            "plants": [{"id": "P2", "name": "Plant 2 (Stamping)", "warehouse": "Plant 2 Stores"}],
            "lines": [{"id": "L", "name": "Press Line A", "plant": "P2", "rate_per_hour": 400},
                      {"id": "M", "name": "Press Line B", "plant": "P2", "rate_per_hour": 300}],
            "machines": [{"id": "PR-A1", "name": "Press A1", "line": "L", "kind": "press",
                          "spindle_part": "P-1",
                          "history": [{"date": "2026-02-01", "by": "Dana Ruiz",
                                       "work": "Spindle bearing inspection: fine"},
                                      {"date": "2026-01-01", "by": "Dana Ruiz",
                                       "work": "Annual PM: fine"}]}],
            "technicians": [{"id": "T-1", "name": "Dana Ruiz"}],
            "main_drives": {"press": {"shaft_rpm": 300, "bearing": {"designation": "B"},
                                      "defect_orders": {"ftf": 0.4, "bpfo": 9.596,
                                                        "bpfi": 12.4, "bsf": 3.7}}},
            "items": [{"code": "P-1", "name": "Spindle", "group": "MRO spares", "uom": "Nos",
                       "price": 4400},
                      {"code": "RC-1", "name": "Bracket", "group": "Stamped parts", "uom": "Nos",
                       "price": 6.5, "weight_kg": 2.0},
                      {"code": "ST", "name": "Coil", "group": "Raw material", "uom": "Kg",
                       "price": 1.1}],
            "suppliers": [{"name": "Good", "group": "Approved", "approved": true,
                           "history": [{"item": "P-1", "rate": 4180, "days_ago": 10}]},
                          {"name": "Other", "group": "Approved", "approved": true, "history": []}],
            "customers": [{"name": "C", "terms": "Net 30"}],
            "orders": [{"id": "A-1", "customer": "C", "line": "L", "item": "RC-1", "qty": 1000,
                        "due_in_days": 2},
                       {"id": "Z", "customer": "C", "line": "M", "item": "RC-1", "qty": 10,
                        "due_in_days": 3}],
        })
    }

    fn with<T>(f: impl FnOnce(&Company) -> T) -> T {
        let (data, extra) = (data(), company_extra());
        f(&Company {
            data: &data,
            extra: &extra,
            today: Day::from_epoch(20_000),
        })
    }

    #[test]
    fn the_manual_holds_the_peak_that_means_the_outer_race() {
        let n = manual_note(&data()).unwrap();
        let content = n["content"].as_str().unwrap();
        assert!(
            content.contains("Outer race (BPFO)</td><td>9.596</td><td>47.98 Hz"),
            "{content}"
        );
        assert!(content.contains("Replacement part P-1"), "{content}");
    }

    #[test]
    fn every_record_is_marked_with_the_company_and_the_stock_gap_is_said() {
        with(|c| {
            assert_eq!(c.company_doc()["abbr"], "RC");
            let notes = notes(c).unwrap();
            assert_eq!(notes.len(), 3);
            let stock = notes[1]["content"].as_str().unwrap();
            assert!(stock.contains("P-1: 0 on hand, minimum 1"), "{stock}");
            assert!(notes[2]["content"].as_str().unwrap().contains("invented"));
        });
    }

    #[test]
    fn the_press_lines_are_workstations_on_the_plant_floor() {
        with(|c| {
            let docs = c.workstations().unwrap();
            let names: Vec<_> = docs
                .iter()
                .filter_map(|d| d["workstation_name"].as_str())
                .collect();
            assert_eq!(names, ["Press Line A", "Press Line B"]);
            assert_eq!(docs[1]["plant_floor"], "Plant 2 (Stamping)");
            assert!(docs.iter().any(|d| d["doctype"] == "Operation"));
        });
    }

    #[test]
    fn a_work_order_runs_on_its_own_line_and_a_sales_order_carries_the_plants_id() {
        with(|c| {
            let orders: Vec<_> = list(c.data, "orders").collect();
            let so = c.sales_order(orders[1]).unwrap();
            assert_eq!(so["po_no"], "Z");
            assert_eq!(so["delivery_date"], "2024-10-07");
            assert_eq!(so["transaction_date"], "2024-09-20");
            let wo = c.work_order(orders[1], "SO", "BOM").unwrap();
            assert_eq!(wo["operations"][0]["workstation"], "Press Line B");
            assert_eq!(wo["operations"][0]["time_in_mins"], 0.2);
            assert_eq!(wo["fg_warehouse"], "Plant 2 Finished Goods - RC");
        });
    }

    #[test]
    fn a_bom_uses_coil_with_scrap_and_the_line_its_first_order_is_on() {
        with(|c| {
            let boms = c.boms().unwrap();
            assert_eq!(boms.len(), 1);
            assert_eq!(boms[0]["items"][0]["qty"], 2.5);
            assert_eq!(boms[0]["operations"][0]["workstation"], "Press Line A");
        });
    }

    #[test]
    fn prices_hold_the_lists_and_what_each_supplier_charged() {
        with(|c| {
            let prices = c.prices();
            assert_eq!(prices.len(), 4);
            let paid = prices.iter().find(|p| p["supplier"] == "Good").unwrap();
            assert_eq!(paid["price_list"], "Standard Buying");
            assert_eq!(paid["valid_from"], "2024-09-24");
            let sold = prices.iter().find(|p| p["item_code"] == "RC-1").unwrap();
            assert_eq!(sold["price_list"], "Standard Selling");
        });
    }

    #[test]
    fn a_press_has_a_plan_per_kind_of_work_and_a_log_per_line_of_history() {
        with(|c| {
            let m = crate::facts::machine(c.data).unwrap();
            let plan = c.maintenance(m, "ASSET-1").unwrap();
            let tasks = plan["asset_maintenance_tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 2);
            assert_eq!(
                tasks[0]["maintenance_task"],
                "Annual preventive maintenance"
            );
            assert_eq!(tasks[0]["assign_to"], "dana.ruiz@ridgeline.example");
            let saved = json!({"name": "ASSET-1", "asset_maintenance_tasks": [
                {"name": "r1", "maintenance_task": "Annual preventive maintenance"},
                {"name": "r2", "maintenance_task": "Spindle bearing inspection"}]});
            let logs = c.maintenance_logs(m, &saved).unwrap();
            assert_eq!((logs.len(), &logs[0]["task"]), (2, &json!("r2")));
        });
    }

    #[test]
    fn the_failing_presss_part_has_a_shelf_minimum_and_no_stock() {
        with(|c| {
            let items = c.items().unwrap();
            let part = items.iter().find(|i| i["item_code"] == "P-1").unwrap();
            let level = &part["reorder_levels"][0];
            assert_eq!(level["warehouse"], "Plant 2 Stores - RC");
            let stock = c.opening_stock();
            // The real file names items this cut-down data doesn't have; the real one is read below.
            assert!(stock.is_err());
        });
    }

    #[test]
    fn the_company_file_stocks_only_what_the_example_sells_and_not_the_failing_part() {
        let extra = company_extra();
        let stocked: Vec<_> = list(&extra, "opening_stock").map(|s| &s["item"]).collect();
        assert!(
            !stocked.contains(&&json!("SP-SPN-800")),
            "the part must be out of stock"
        );
        assert!(stocked.contains(&&json!("SP-SPN-600")));
        let fictional = extra["fictional"].as_str().unwrap();
        assert!(fictional.contains("invented"), "{fictional}");
    }

    #[test]
    fn many_writes_at_once_all_run_and_the_first_error_is_kept() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let r = rt.block_on(each_at_once((0..20).collect(), |n: usize| {
            let done = done.clone();
            async move {
                done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                anyhow::ensure!(n != 7, "seven");
                Ok(())
            }
        }));
        assert_eq!(r.unwrap_err().to_string(), "seven");
        assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 20);
    }

    #[test]
    fn a_crm_opportunity_is_the_order_and_a_contact_is_at_the_account() {
        with(|c| {
            let order = list(c.data, "orders").next().unwrap();
            let deal = crm::opportunity(c, order, "acc").unwrap();
            assert_eq!(deal["name"], "A-1 RC-1 x 1000");
            assert_eq!(deal["closeDate"], "2024-10-06");
            assert_eq!(deal["amount"]["amountMicros"], 6_500_000_000u64);
            let p = crm::person(
                &json!({"first": "A", "last": "C", "email": "a@c.example",
                                        "title": "Buyer"}),
                "acc",
            );
            assert_eq!(p["companyId"], "acc");
            assert_eq!(p["emails"]["primaryEmail"], "a@c.example");
        });
    }
}
