//! Compact reports read from the ERP, for a deployment that wants summaries instead of raw
//! queries: the orders planned from a batch, the stock of every lot, the lots that could serve a
//! customer on a day, and the production schedule. They only read, through the ERP's REST API, so
//! each is attributed to the ERP; the raw query tools stay available beside them.
use anyhow::Result;
use arena_part::proto::{Effect, ToolSchema};
use serde_json::{json, Value};

use crate::data::Data;
use crate::days::day;
use crate::erp::Erp;

fn schema(name: &str, description: &str, props: Value, required: &[&str]) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": props, "required": required}),
        effect: Effect::Read,
    }
}

/// Each report: its name, what it gives and its text arguments (all required).
const REPORTS: [(&str, &str, &[&str]); 4] = [
    (
        "affected_orders",
        "Report: the open sales orders planned from a batch, earliest due first, with \
      each customer's minimum remaining shelf life.",
        &["batch"],
    ),
    (
        "stock_report",
        "Report: every Corvexa lot with its warehouse, QA status, expiry, stock, quantity \
      committed to open orders, free quantity and reservation.",
        &[],
    ),
    (
        "eligible_lots",
        "Report: the released lots with free stock and enough shelf life left on `arrives` \
      (YYYY-MM-DD) for the customer's minimum.",
        &["customer", "arrives"],
    ),
    (
        "production_schedule",
        "Report: the work orders, with planned batch, quantity, planned start and \
      status.",
        &[],
    ),
];

pub fn schemas() -> Vec<ToolSchema> {
    let text = json!({"type": "string"});
    let report = |(name, description, args): &(&str, &str, &[&str])| {
        let props: serde_json::Map<String, Value> =
            args.iter().map(|a| (a.to_string(), text.clone())).collect();
        schema(name, description, Value::Object(props), args)
    };
    REPORTS.iter().map(report).collect()
}

pub fn has(name: &str) -> bool {
    schemas().iter().any(|s| s.name == name)
}

/// A report's answer and the ERP keys it read; `Err` for a refusal (bad arguments, a refusal of
/// the ERP's).
pub type Report = Result<(Value, Vec<String>), String>;

pub async fn run(erp: &Erp, d: &Data, name: &str, args: &Value) -> Report {
    let text = |k: &str| {
        args[k]
            .as_str()
            .map(str::to_string)
            .ok_or(format!("{name}: {k} is required (text)"))
    };
    let out = match name {
        "affected_orders" => affected_orders(erp, &text("batch")?).await,
        "stock_report" => stock(erp, d)
            .await
            .map(|(rows, keys)| (json!({"lots": rows}), keys)),
        "eligible_lots" => eligible(erp, d, &text("customer")?, &text("arrives")?).await,
        "production_schedule" => schedule(erp).await,
        other => return Err(format!("no report {other}")),
    };
    out.map_err(|e| format!("{e:#}"))
}

fn open_orders() -> Value {
    json!([
        ["docstatus", "=", 1],
        ["status", "not in", ["Completed", "Closed", "Cancelled"]]
    ])
}

fn keys(doctype: &str, rows: &[Value]) -> Vec<String> {
    let name = |r: &Value| r["name"].as_str().map(|n| format!("{doctype}/{n}"));
    rows.iter().filter_map(name).collect()
}

async fn affected_orders(erp: &Erp, batch: &str) -> Result<(Value, Vec<String>)> {
    let mut filters = open_orders();
    filters
        .as_array_mut()
        .expect("a list")
        .push(json!(["planned_batch", "=", batch]));
    let fields = ["name", "po_no", "customer", "delivery_date", "total_qty"];
    let mut rows = erp.list("Sales Order", &filters, &fields).await?;
    let customers = erp
        .list(
            "Customer",
            &json!([]),
            &["name", "min_shelf_life_days", "territory"],
        )
        .await?;
    for r in &mut rows {
        let c = customers.iter().find(|c| c["name"] == r["customer"]);
        r["customer_min_shelf_life_days"] =
            c.map_or(Value::Null, |c| c["min_shelf_life_days"].clone());
        r["territory"] = c.map_or(Value::Null, |c| c["territory"].clone());
    }
    rows.sort_by_key(|r| (r["delivery_date"].to_string(), r["po_no"].to_string()));
    let read = keys("Sales Order", &rows);
    Ok((json!({"batch": batch, "orders": rows}), read))
}

/// A lot's line of the stock report, `held` of it committed to open orders.
fn lot_row(l: &Value, held: f64) -> Value {
    let on_hand = l["batch_qty"].as_f64().unwrap_or(0.0);
    json!({"batch": l["name"], "warehouse": l["warehouse"], "qa_status": l["qa_status"],
           "expiry": l["expiry_date"], "on_hand": on_hand, "committed": held,
           "free": (on_hand - held).max(0.0), "reserved_for": l["reserved_for"]})
}

/// Every lot of the product in the ERP, with the fields the stock report reads.
pub async fn lots(erp: &Erp, d: &Data) -> Result<Vec<Value>> {
    let fields = [
        "name",
        "expiry_date",
        "qa_status",
        "warehouse",
        "reserved_for",
        "batch_qty",
    ];
    erp.list("Batch", &json!([["item", "=", d.product.code]]), &fields)
        .await
}

/// The open sales orders, with `fields`.
pub async fn orders(erp: &Erp, fields: &[&str]) -> Result<Vec<Value>> {
    erp.list("Sales Order", &open_orders(), fields).await
}

/// The stock report's lines: each lot with what the open `orders` hold of it.
pub fn stock_rows(lots: &[Value], orders: &[Value]) -> Vec<Value> {
    let committed = |batch: &Value| -> f64 {
        let of = orders.iter().filter(|o| o["planned_batch"] == *batch);
        of.filter_map(|o| o["total_qty"].as_f64()).sum::<f64>() + 0.0
    };
    lots.iter()
        .map(|l| lot_row(l, committed(&l["name"])))
        .collect()
}

/// Every lot of the product with what open orders hold of it.
async fn stock(erp: &Erp, d: &Data) -> Result<(Vec<Value>, Vec<String>)> {
    let lots = lots(erp, d).await?;
    let orders = orders(erp, &["planned_batch", "total_qty"]).await?;
    let read = keys("Batch", &lots);
    Ok((stock_rows(&lots, &orders), read))
}

async fn eligible(
    erp: &Erp,
    d: &Data,
    customer: &str,
    arrives: &str,
) -> Result<(Value, Vec<String>)> {
    let on = day(arrives).ok_or_else(|| anyhow::anyhow!("arrives must be a date (YYYY-MM-DD)"))?;
    let c = erp.get("Customer", customer).await?;
    let min = c["min_shelf_life_days"].as_i64().unwrap_or(0);
    let (rows, mut read) = stock(erp, d).await?;
    let left =
        |r: &Value| day(r["expiry"].as_str().unwrap_or_default()).map_or(i64::MIN, |e| e - on);
    let fits = |r: &&Value| {
        r["qa_status"] == "Released" && r["free"].as_f64() > Some(0.0) && left(r) >= min
    };
    let lots: Vec<Value> = (rows.iter().filter(fits))
        .map(|r| json!({"remaining_days": left(r), "lot": r}))
        .collect();
    read.push(format!("Customer/{customer}"));
    Ok((
        json!({"customer": customer, "min_shelf_life_days": min, "arrives": arrives, "lots": lots}),
        read,
    ))
}

/// The work orders not cancelled.
pub async fn work_orders(erp: &Erp) -> Result<Vec<Value>> {
    let fields = [
        "name",
        "planned_batch",
        "production_item",
        "qty",
        "planned_start_date",
        "status",
        "docstatus",
    ];
    erp.list("Work Order", &json!([["docstatus", "<", 2]]), &fields)
        .await
}

async fn schedule(erp: &Erp) -> Result<(Value, Vec<String>)> {
    let rows = work_orders(erp).await?;
    let read = keys("Work Order", &rows);
    Ok((json!({"work_orders": rows}), read))
}
