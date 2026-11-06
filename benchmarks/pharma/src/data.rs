//! The benchmark's data (`data/larchmont.json`): the fictional company, its lots, customers and
//! orders, and the slices the simulated LIMS and carriers are configured with. Everything the seed
//! writes and the checks expect comes from here.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Data {
    pub fictional: String,
    pub company: String,
    pub abbr: String,
    pub currency: String,
    pub country: String,
    pub today: String,
    pub product: Product,
    pub components: Vec<Component>,
    pub component_stock: BTreeMap<String, f64>,
    pub sites: Vec<Site>,
    pub workstations: Vec<Workstation>,
    pub lots: Vec<Lot>,
    pub work_orders: Vec<WorkOrder>,
    pub incident: Incident,
    pub customers: Vec<Customer>,
    pub orders: Vec<Order>,
    pub lims: Value,
    pub logistics: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    pub code: String,
    pub name: String,
    pub group: String,
    pub uom: String,
    pub price: f64,
    pub shelf_life_days: i64,
    pub storage: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub code: String,
    pub name: String,
    pub group: String,
    pub uom: String,
    pub price: f64,
    pub per_unit: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub place: String,
    pub warehouses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workstation {
    pub name: String,
    pub description: String,
    pub operation: String,
    pub minutes_per_unit: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lot {
    pub batch: String,
    pub manufactured: String,
    pub expiry: String,
    pub site: String,
    pub warehouse: String,
    pub qty: u64,
    pub qa_status: String,
    #[serde(default)]
    pub reserved_for: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkOrder {
    pub batch: String,
    pub qty: u64,
    pub start: String,
    pub status: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Incident {
    pub batch: String,
    pub sample: String,
    pub next_batch: String,
    pub restock_date: String,
    pub restock_window_days: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Contact {
    pub first: String,
    pub last: String,
    pub title: String,
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Customer {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub region: String,
    pub city: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    pub min_shelf_life_days: i64,
    pub domain: String,
    pub contacts: Vec<Contact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Order {
    pub po: String,
    pub customer: String,
    pub due: String,
    pub qty: u64,
    pub planned_batch: String,
}

/// The data, read once from the file built into the program.
pub fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/larchmont.json"))
            .expect("data/larchmont.json has the benchmark's shape (a test reads it)")
    })
}

impl Data {
    pub fn customer(&self, name: &str) -> Option<&Customer> {
        self.customers.iter().find(|c| c.name == name)
    }

    pub fn lot(&self, batch: &str) -> Option<&Lot> {
        self.lots.iter().find(|l| l.batch == batch)
    }

    pub fn order(&self, po: &str) -> Option<&Order> {
        self.orders.iter().find(|o| o.po == po)
    }

    pub fn site(&self, id: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.id == id)
    }

    /// The orders the incident affects: those planned from the failing batch, earliest due first.
    pub fn affected(&self) -> Vec<&Order> {
        let mut hit: Vec<&Order> = (self.orders.iter())
            .filter(|o| o.planned_batch == self.incident.batch)
            .collect();
        hit.sort_by(|a, b| (&a.due, &a.po).cmp(&(&b.due, &b.po)));
        hit
    }

    /// What of `lot` no other open order is planned from.
    pub fn free_qty(&self, lot: &Lot) -> u64 {
        let committed: u64 = (self.orders.iter())
            .filter(|o| o.planned_batch == lot.batch)
            .map(|o| o.qty)
            .sum();
        lot.qty.saturating_sub(committed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_holds_the_incident() {
        let d = data();
        assert!(d.fictional.contains("invented"));
        let hit = d.affected();
        assert_eq!(hit.len(), 8);
        assert_eq!(hit[0].due, "2026-11-10");
        assert!(hit.iter().all(|o| d.customer(&o.customer).is_some()));
        let free = |b: &str| d.free_qty(d.lot(b).unwrap());
        assert_eq!((free("CVX-2604"), free("CVX-2598")), (1000, 2500));
        assert!(d.orders.iter().all(|o| d.customer(&o.customer).is_some()));
        assert!(d.customers.len() >= 24 && d.orders.len() >= 40);
    }
}
