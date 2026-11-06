//! The world's end state, `{part: {key: doc}}`, read the way the checks need it: the approvals'
//! decided changes, the ERP's batches and orders, the carriers' shipments and notices, the CRM.
use serde_json::Value;

use crate::days::day;

pub struct World<'a>(pub &'a Value);

/// A shipment booked with a carrier, as the checks read it.
pub struct Booking<'a> {
    pub doc: &'a Value,
    /// The order's id (its customer PO number), however the booking named it.
    pub order: &'a str,
    pub batch: &'a str,
    pub customer: &'a str,
    pub carrier: &'a str,
    pub origin: &'a str,
    pub qty: u64,
    pub ship: Option<i64>,
    pub arrives: Option<i64>,
}

impl<'a> World<'a> {
    /// The documents of `part` whose keys start with `<kind>/`, in key order.
    pub fn docs(&self, part: &str, kind: &str) -> Vec<&'a Value> {
        let prefix = format!("{kind}/");
        let mut docs: Vec<(&String, &Value)> = (self.0[part].as_object().into_iter().flatten())
            .filter(|(k, _)| k.starts_with(&prefix))
            .collect();
        docs.sort_by_key(|(k, _)| k.as_str());
        docs.into_iter().map(|(_, d)| d).collect()
    }

    /// The changes of `action` a person approved.
    pub fn approved(&self, action: &str) -> Vec<&'a Value> {
        let approved = |c: &&Value| {
            let d = &c["decision"];
            c["action"] == action && d["approved"] == true && d["by"] == "person"
        };
        self.docs("approvals", "change")
            .into_iter()
            .filter(approved)
            .collect()
    }

    /// Whether a person approved a change of `action` whose payload names `batch`.
    pub fn approved_for(&self, action: &str, batch: &str) -> bool {
        self.approved(action)
            .iter()
            .any(|c| c["payload"]["batch"] == batch)
    }

    /// The change `id`, approved by a person, of `action`.
    pub fn is_approved(&self, id: &Value, action: &str) -> bool {
        self.approved(action).iter().any(|c| c["id"] == *id)
    }

    /// The order id (a sales order's `po_no`) of an order named by it or by its ERP sales order.
    pub fn order_id(&self, named: &'a Value) -> &'a str {
        let name = named.as_str().unwrap_or_default();
        let sales_order = &self.0["erp"][format!("Sales Order/{name}")];
        sales_order["po_no"].as_str().unwrap_or(name)
    }

    /// The batch's QA status in the ERP at the end.
    pub fn qa_status(&self, batch: &str) -> &'a str {
        let doc = &self.0["erp"][format!("Batch/{batch}")];
        doc["qa_status"].as_str().unwrap_or_default()
    }

    pub fn bookings(&self) -> Vec<Booking<'a>> {
        let text = |d: &'a Value, f: &str| d[f].as_str().unwrap_or_default();
        let shipments = self.docs("logistics", "shipment");
        shipments
            .into_iter()
            .map(|d| Booking {
                doc: d,
                order: self.order_id(&d["order"]),
                batch: text(d, "batch"),
                customer: text(d, "customer"),
                carrier: text(d, "carrier"),
                origin: text(d, "origin"),
                qty: d["qty"].as_u64().unwrap_or(0),
                ship: day(text(d, "ship_date")),
                arrives: day(text(d, "arrives")),
            })
            .collect()
    }

    /// The latest notice sent about `order` to `customer`.
    pub fn notice(&self, order: &str, customer: &str) -> Option<&'a Value> {
        let notices = self.docs("logistics", "notice");
        let about = |n: &&&Value| self.order_id(&n["order"]) == order && n["customer"] == customer;
        notices.iter().rev().find(about).copied()
    }
}
