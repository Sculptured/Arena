//! What the twin reads: the ERP's lots, open orders, work orders and deviations (at once, with the
//! reports' readers), and the other parts' docs as arenad passed them.
use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::data::Data;
use crate::erp::Erp;
use crate::reports;

/// The world at one moment, as the twin needs it.
pub struct Snapshot {
    /// The stock report's lines: `{batch, warehouse, qa_status, expiry, on_hand, committed, free}`.
    pub lots: Vec<Value>,
    /// The open sales orders: `{name, po_no, customer, delivery_date, total_qty, planned_batch}`.
    pub orders: Vec<Value>,
    pub work_orders: Vec<Value>,
    /// The ERP's non-conformances: `{name, subject, status}`.
    pub deviations: Vec<Value>,
    /// `{part: {key: doc}}`: the docs arenad passed, and the ERP's batches and orders as the checks
    /// read them (`Batch/<name>` with `qa_status`, `Sales Order/<name>` with `po_no`).
    pub docs: Value,
}

const ORDER_FIELDS: [&str; 6] = [
    "name",
    "po_no",
    "customer",
    "delivery_date",
    "total_qty",
    "planned_batch",
];

impl Snapshot {
    pub async fn read(erp: &Erp, d: &Data, docs: &Map<String, Value>) -> Result<Snapshot> {
        let (deviation_fields, all) = (["name", "subject", "status"], json!([]));
        let (lots, orders, work_orders, deviations) = tokio::try_join!(
            reports::lots(erp, d),
            reports::orders(erp, &ORDER_FIELDS),
            reports::work_orders(erp),
            erp.list("Non Conformance", &all, &deviation_fields),
        )?;
        Ok(Snapshot::of(lots, orders, work_orders, deviations, docs))
    }

    /// A snapshot of what was read (`lots` as the ERP lists them).
    pub fn of(
        lots: Vec<Value>,
        orders: Vec<Value>,
        work_orders: Vec<Value>,
        deviations: Vec<Value>,
        docs: &Map<String, Value>,
    ) -> Snapshot {
        let mut docs = docs.clone();
        docs.insert("erp".into(), erp_docs(&lots, &orders));
        Snapshot {
            lots: reports::stock_rows(&lots, &orders),
            orders,
            work_orders,
            deviations,
            docs: Value::Object(docs),
        }
    }
}

/// The ERP's batches and orders keyed as the ERP part's `inspect` keys them.
fn erp_docs(lots: &[Value], orders: &[Value]) -> Value {
    let mut out = Map::new();
    for l in lots {
        let key = format!("Batch/{}", super::text(&l["name"]));
        out.insert(key, json!({"qa_status": l["qa_status"]}));
    }
    for o in orders {
        let key = format!("Sales Order/{}", super::text(&o["name"]));
        out.insert(key, json!({"po_no": o["po_no"]}));
    }
    Value::Object(out)
}
