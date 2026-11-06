//! Writes the fictional company into the ERP and the CRM through their REST APIs, once, into the
//! base world every scenario starts from. The LIMS and the carriers start from their config.
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::data::Data;
use crate::erp::{name_of, Erp};
use crate::erp_docs as docs;

/// How many sales orders are written at once.
const PARALLEL: usize = 5;

async fn insert_all(erp: &Erp, list: &[Value]) -> Result<()> {
    for doc in list {
        erp.insert(doc).await?;
    }
    Ok(())
}

async fn insert_missing_all(erp: &Erp, list: &[Value]) -> Result<()> {
    for doc in list {
        erp.insert_missing(doc).await?;
    }
    Ok(())
}

/// The company and the calendar year it books in, the company as the site's default.
async fn company(erp: &Erp, d: &Data) -> Result<()> {
    let year = &d.today[..4];
    if erp
        .names("Fiscal Year", &json!({"year": year}))
        .await?
        .is_empty()
    {
        let fy = json!({"doctype": "Fiscal Year", "year": year, "year_start_date": format!("{year}-01-01"),
                        "year_end_date": format!("{year}-12-31")});
        erp.insert(&fy).await?;
    }
    erp.insert(&docs::company(d)).await?;
    let defaults = json!({"default_company": d.company});
    erp.update("Global Defaults", "Global Defaults", &defaults)
        .await?;
    let batches = json!({"enable_serial_and_batch_no_for_item": 1});
    erp.update("Stock Settings", "Stock Settings", &batches)
        .await?;
    Ok(())
}

/// The masters: fields, warehouses, items, prices, the line, the BOM; the BOM's name.
async fn masters(erp: &Erp, d: &Data) -> Result<String> {
    insert_all(erp, &docs::custom_fields())
        .await
        .context("custom fields")?;
    insert_all(erp, &docs::warehouses(d))
        .await
        .context("warehouses")?;
    insert_missing_all(erp, &docs::item_groups(d)).await?;
    insert_all(erp, &docs::items(d)).await.context("items")?;
    insert_all(erp, &docs::prices(d)).await?;
    insert_all(erp, &docs::workstations(d))
        .await
        .context("workstations")?;
    let bom = erp.submit_new(&docs::bom(d)).await.context("the BOM")?;
    Ok(name_of(&bom)?.to_string())
}

/// The lots and their stock.
async fn stock(erp: &Erp, d: &Data) -> Result<()> {
    for lot in &d.lots {
        erp.insert(&docs::batch(d, lot)).await.context("a batch")?;
    }
    for entry in docs::opening_stock(d) {
        erp.submit_new(&entry).await.context("the opening stock")?;
    }
    Ok(())
}

/// The customers and their sales orders, submitted, several at once.
async fn customers_and_orders(erp: &Arc<Erp>, d: &'static Data) -> Result<()> {
    insert_missing_all(erp, &docs::customer_masters()).await?;
    for c in &d.customers {
        erp.insert(&docs::customer(c)).await.context("a customer")?;
    }
    for chunk in d.orders.chunks(PARALLEL) {
        let mut running = tokio::task::JoinSet::new();
        for o in chunk {
            let erp = erp.clone();
            running.spawn(async move { erp.submit_new(&docs::sales_order(d, o)).await.map(drop) });
        }
        while let Some(done) = running.join_next().await {
            done.context("a sales order task ended")?
                .context("a sales order")?;
        }
    }
    Ok(())
}

/// The work orders: the one filling now submitted, the next ones planned (drafts).
async fn work_orders(erp: &Erp, d: &Data, bom: &str) -> Result<()> {
    for w in &d.work_orders {
        let doc = docs::work_order(d, w, bom);
        match w.status.as_str() {
            "submitted" => erp.submit_new(&doc).await.map(drop),
            _ => erp.insert(&doc).await.map(drop),
        }
        .with_context(|| format!("the work order of {}", w.batch))?;
    }
    Ok(())
}

/// Writes the company into the ERP (`erp` is its `connect` info).
pub async fn seed_erp(erp: &Value, d: &'static Data) -> Result<()> {
    let erp = Arc::new(Erp::connect(erp)?);
    company(&erp, d).await.context("the company")?;
    let bom = masters(&erp, d).await?;
    stock(&erp, d).await?;
    customers_and_orders(&erp, d).await?;
    work_orders(&erp, d, &bom).await?;
    erp.insert(&docs::quality_procedure())
        .await
        .context("the quality procedure")?;
    erp.insert(&docs::fictional_note(d)).await.map(drop)
}

/// Loads the company into the ERP and the CRM.
pub async fn seed(erp: &Value, crm: &Value, d: &'static Data) -> Result<()> {
    seed_erp(erp, d).await.context("the ERP")?;
    crate::crm::seed(crm, d).await.context("the CRM")
}
