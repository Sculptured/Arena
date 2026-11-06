//! Seeds the ERP of a site that is already running and reads the reports back. `live_`: it needs
//! an ERPNext site of the erpnext integration, whose `connect` info is in the file
//! `PHARMA_ERP_CONNECT` names (it writes the company into that site and leaves it there).
//! Run: `PHARMA_ERP_CONNECT=erp.json cargo test -p arena-pharma --test live_seed -- --ignored`.
use arena_pharma::{data::data, erp::Erp, reports, seed};
use serde_json::{json, Value};

fn connect_info() -> Option<Value> {
    let path = std::env::var("PHARMA_ERP_CONNECT").ok()?;
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

async fn report(erp: &Erp, name: &str, args: Value) -> Value {
    let out = reports::run(erp, data(), name, &args).await.unwrap().0;
    println!("{name}: {out}");
    out
}

/// The free quantity of `batch` in the stock report.
fn free(stock: &Value, batch: &str) -> Option<f64> {
    let lots = stock["lots"].as_array()?;
    lots.iter().find(|l| l["batch"] == batch)?["free"].as_f64()
}

#[tokio::test]
#[ignore]
async fn live_seed_into_a_given_site() {
    let Some(info) = connect_info() else {
        eprintln!("skipped: no PHARMA_ERP_CONNECT");
        return;
    };
    seed::seed_erp(&info, data()).await.unwrap();
    let erp = Erp::connect(&info).unwrap();
    let stock = report(&erp, "stock_report", json!({})).await;
    assert_eq!(
        (free(&stock, "CVX-2604"), free(&stock, "CVX-2598")),
        (Some(1000.0), Some(2500.0))
    );
    let hit = report(&erp, "affected_orders", json!({"batch": "CVX-2611"})).await;
    assert_eq!(hit["orders"].as_array().unwrap().len(), 8);
    let who = json!({"customer": "Pinecrest Wholesale Drug", "arrives": "2026-11-12"});
    let eligible = report(&erp, "eligible_lots", who).await;
    let lots: Vec<&Value> = eligible["lots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| &l["lot"]["batch"])
        .collect();
    assert_eq!(lots, [&json!("CVX-2598"), &json!("CVX-2604")]);
    let schedule = report(&erp, "production_schedule", json!({})).await;
    assert_eq!(schedule["work_orders"].as_array().unwrap().len(), 3);
}
