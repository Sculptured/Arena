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

#[tokio::test]
#[ignore]
async fn live_seed_into_a_given_site() {
    let Some(info) = connect_info() else {
        eprintln!("skipped: no PHARMA_ERP_CONNECT");
        return;
    };
    if std::env::var_os("PHARMA_SKIP_SEED").is_none() {
        seed::seed_erp(&info, data()).await.unwrap();
    }
    let erp = Erp::connect(&info).unwrap();
    let run = |name: &'static str, args: Value| {
        let erp = &erp;
        async move { reports::run(erp, data(), name, &args).await.unwrap().0 }
    };
    let stock = run("stock_report", json!({})).await;
    println!("{}", serde_json::to_string_pretty(&stock).unwrap());
    let lot = |b: &str| {
        stock["lots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["batch"] == b)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        (
            lot("CVX-2604")["free"].as_f64(),
            lot("CVX-2598")["free"].as_f64()
        ),
        (Some(1000.0), Some(2500.0))
    );
    let hit = run("affected_orders", json!({"batch": "CVX-2611"})).await;
    assert_eq!(hit["orders"].as_array().unwrap().len(), 8, "{hit}");
    let eligible = run(
        "eligible_lots",
        json!({"customer": "Pinecrest Wholesale Drug", "arrives": "2026-11-12"}),
    )
    .await;
    println!("{eligible}");
    let schedule = run("production_schedule", json!({})).await;
    assert_eq!(
        schedule["work_orders"].as_array().unwrap().len(),
        3,
        "{schedule}"
    );
}
