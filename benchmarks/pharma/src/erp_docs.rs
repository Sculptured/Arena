//! The fictional company's ERP records as documents: custom fields, warehouses, items and prices,
//! the filling line's workstations, the BOM, the lots with their stock, customers, sales orders,
//! work orders and the quality procedure a deviation links to. Each function only builds
//! documents; `seed` writes them.
use serde_json::{json, Value};

use crate::data::{Customer, Data, Lot, Order};
use crate::days::{day, iso};

const HOSPITALS: &str = "Hospitals";
const WHOLESALERS: &str = "Wholesalers";
/// The quality procedure a deviation (Non Conformance) of an EM excursion links to.
pub const EM_PROCEDURE: &str = "EM excursion investigation (SOP-QA-031)";
/// The day the opening stock is posted.
const OPENING: &str = "2026-11-01";

/// `<name> - LB`, as ERPNext names a company's warehouses.
pub fn of_company(d: &Data, name: &str) -> String {
    format!("{name} - {}", d.abbr)
}

/// The warehouse `role` (`Released`, `Quarantine`, ...) of the site with id `site`.
pub fn warehouse(d: &Data, site: &str, role: &str) -> String {
    let name = d.site(site).map_or(site, |s| s.name.as_str());
    of_company(d, &format!("{name} {role}"))
}

pub fn company(d: &Data) -> Value {
    json!({"doctype": "Company", "company_name": d.company, "abbr": d.abbr,
           "default_currency": d.currency, "country": d.country,
           "create_chart_of_accounts_based_on": "Standard Template", "chart_of_accounts": "Standard"})
}

fn custom_field(dt: &str, name: &str, label: &str, kind: &str, options: &str) -> Value {
    json!({"doctype": "Custom Field", "dt": dt, "fieldname": name, "label": label, "fieldtype": kind,
           "options": options, "in_list_view": 1})
}

/// What the ERP keeps beyond its standard fields: a lot's QA status, where it is and whom it is
/// reserved for; a customer's minimum remaining shelf life; the batch an order or a work order
/// is planned from.
pub fn custom_fields() -> Vec<Value> {
    vec![
        custom_field(
            "Batch",
            "qa_status",
            "QA status",
            "Select",
            "Quarantined\nReleased\nOn Hold\nRejected",
        ),
        custom_field("Batch", "warehouse", "Warehouse", "Link", "Warehouse"),
        custom_field("Batch", "reserved_for", "Reserved for", "Data", ""),
        custom_field(
            "Customer",
            "min_shelf_life_days",
            "Minimum remaining shelf life (days)",
            "Int",
            "",
        ),
        custom_field("Sales Order", "planned_batch", "Planned batch", "Data", ""),
        custom_field("Work Order", "planned_batch", "Planned batch", "Data", ""),
    ]
}

/// Each site's group, then its warehouses.
pub fn warehouses(d: &Data) -> Vec<Value> {
    let mut out = vec![];
    for s in &d.sites {
        out.push(
            json!({"doctype": "Warehouse", "warehouse_name": s.name, "is_group": 1,
                        "company": d.company, "parent_warehouse": of_company(d, "All Warehouses")}),
        );
        for role in &s.warehouses {
            out.push(json!({"doctype": "Warehouse", "company": d.company,
                            "warehouse_name": format!("{} {role}", s.name),
                            "parent_warehouse": of_company(d, &s.name)}));
        }
    }
    out
}

pub fn item_groups(d: &Data) -> Vec<Value> {
    let mut groups: Vec<&str> = d.components.iter().map(|c| c.group.as_str()).collect();
    groups.push(&d.product.group);
    groups.sort_unstable();
    groups.dedup();
    let group = |g: &&str| json!({"doctype": "Item Group", "item_group_name": g, "parent_item_group": "All Item Groups"});
    groups.iter().map(group).collect()
}

/// The product (batch-tracked, with expiry), then its components.
pub fn items(d: &Data) -> Vec<Value> {
    let p = &d.product;
    let mut out = vec![
        json!({"doctype": "Item", "item_code": p.code, "item_name": p.name,
        "description": format!("{}. Store at {}.", p.name, p.storage), "item_group": p.group,
        "stock_uom": p.uom, "is_stock_item": 1, "is_sales_item": 1, "is_purchase_item": 0,
        "has_batch_no": 1, "has_expiry_date": 1, "create_new_batch": 0,
        "shelf_life_in_days": p.shelf_life_days, "include_item_in_manufacturing": 1,
        "item_defaults": [{"company": d.company, "default_warehouse": warehouse(d, "US", "Released")}]}),
    ];
    out.extend(d.components.iter().map(|c| {
        json!({"doctype": "Item", "item_code": c.code, "item_name": c.name, "description": c.name,
               "item_group": c.group, "stock_uom": c.uom, "is_stock_item": 1, "is_sales_item": 0,
               "is_purchase_item": 1, "valuation_rate": c.price, "include_item_in_manufacturing": 1,
               "item_defaults": [{"company": d.company, "default_warehouse": warehouse(d, "US", "Stores")}]})
    }));
    out
}

pub fn prices(d: &Data) -> Vec<Value> {
    let mut out = vec![json!({"doctype": "Item Price", "item_code": d.product.code,
                              "price_list": "Standard Selling", "price_list_rate": d.product.price})];
    out.extend(d.components.iter().map(|c| {
        json!({"doctype": "Item Price", "item_code": c.code, "price_list": "Standard Buying",
               "price_list_rate": c.price})
    }));
    out
}

/// The operations, then the workstations that do them (the MES).
pub fn workstations(d: &Data) -> Vec<Value> {
    let ops = d.workstations.iter().map(|w| {
        json!({"doctype": "Operation", "name": w.operation, "description": w.operation,
               "workstation": w.name})
    });
    let stations = d.workstations.iter().map(|w| {
        json!({"doctype": "Workstation", "workstation_name": w.name, "description": w.description,
               "production_capacity": 1, "warehouse": warehouse(d, "US", "WIP"), "status": "Production"})
    });
    stations.chain(ops).collect()
}

fn operation_rows(d: &Data) -> Vec<Value> {
    let row = |w: &crate::data::Workstation| json!({"operation": w.operation, "workstation": w.name, "time_in_mins": w.minutes_per_unit});
    d.workstations.iter().map(row).collect()
}

pub fn bom(d: &Data) -> Value {
    let items: Vec<Value> = (d.components.iter())
        .map(|c| json!({"item_code": c.code, "qty": c.per_unit}))
        .collect();
    json!({"doctype": "BOM", "item": d.product.code, "company": d.company, "quantity": 1,
           "is_active": 1, "is_default": 1, "rm_cost_as_per": "Valuation Rate", "with_operations": 1,
           "operations": operation_rows(d), "items": items})
}

pub fn batch(d: &Data, lot: &Lot) -> Value {
    json!({"doctype": "Batch", "batch_id": lot.batch, "item": d.product.code,
           "manufacturing_date": lot.manufactured, "expiry_date": lot.expiry,
           "qa_status": lot.qa_status, "warehouse": warehouse(d, &lot.site, &lot.warehouse),
           "reserved_for": lot.reserved_for.clone().unwrap_or_default()})
}

fn receipt(d: &Data, posted: &str, remarks: &str, rows: Vec<Value>) -> Value {
    json!({"doctype": "Stock Entry", "stock_entry_type": "Material Receipt", "purpose": "Material Receipt",
           "company": d.company, "set_posting_time": 1, "posting_date": posted, "remarks": remarks,
           "items": rows})
}

/// The day a lot's stock is received: the day it was made, kept within the books' year and not
/// after the opening (ERPNext refuses a receipt of an expired lot, and one dated in the future).
fn received(lot: &Lot) -> String {
    let first = format!("{}-01-02", &OPENING[..4]);
    let made = lot.manufactured.as_str().max(first.as_str());
    made.min(OPENING).to_string()
}

/// The opening stock: the components in the plant's stores, then each lot in its warehouse.
pub fn opening_stock(d: &Data) -> Vec<Value> {
    let parts: Vec<Value> = (d.components.iter())
        .map(|c| json!({"item_code": c.code, "qty": d.component_stock.get(&c.code).copied().unwrap_or(0.0),
                        "t_warehouse": warehouse(d, "US", "Stores"), "basic_rate": c.price}))
        .collect();
    let mut out = vec![receipt(d, OPENING, "Opening stock: components", parts)];
    out.extend(d.lots.iter().map(|l| {
        let row = json!({"item_code": d.product.code, "qty": l.qty, "batch_no": l.batch,
                         "use_serial_batch_fields": 1, "basic_rate": 410.0,
                         "t_warehouse": warehouse(d, &l.site, &l.warehouse)});
        receipt(
            d,
            &received(l),
            &format!("Opening stock: {}", l.batch),
            vec![row],
        )
    }));
    out
}

pub fn customer_masters() -> Vec<Value> {
    let group = |g: &str| {
        json!({"doctype": "Customer Group", "customer_group_name": g,
               "parent_customer_group": "All Customer Groups"})
    };
    let territory = |t: &str| json!({"doctype": "Territory", "territory_name": t, "parent_territory": "All Territories"});
    vec![
        group(HOSPITALS),
        group(WHOLESALERS),
        territory("United States"),
        territory("Europe"),
    ]
}

pub fn customer(c: &Customer) -> Value {
    let group = if c.kind == "Wholesaler" {
        WHOLESALERS
    } else {
        HOSPITALS
    };
    let territory = if c.region == "EU" {
        "Europe"
    } else {
        "United States"
    };
    let place = format!(
        "{}, {}",
        c.city,
        c.state
            .as_deref()
            .or(c.country.as_deref())
            .unwrap_or_default()
    );
    json!({"doctype": "Customer", "customer_name": c.name, "customer_type": "Company",
           "customer_group": group, "territory": territory, "customer_details": place,
           "min_shelf_life_days": c.min_shelf_life_days})
}

/// The site an order ships from: its planned lot's, else the plant.
fn site_of<'a>(d: &'a Data, o: &Order) -> &'a str {
    d.lot(&o.planned_batch).map_or("US", |l| l.site.as_str())
}

pub fn sales_order(d: &Data, o: &Order) -> Value {
    let store = warehouse(d, site_of(d, o), "Released");
    let ordered = day(&o.due).map_or(d.today.clone(), |due| iso(due - 21));
    json!({"doctype": "Sales Order", "customer": o.customer, "company": d.company, "order_type": "Sales",
           "transaction_date": ordered, "delivery_date": o.due, "po_no": o.po,
           "planned_batch": o.planned_batch, "set_warehouse": store,
           "selling_price_list": "Standard Selling",
           "items": [{"item_code": d.product.code, "qty": o.qty, "delivery_date": o.due,
                      "rate": d.product.price, "warehouse": store}]})
}

pub fn work_order(d: &Data, w: &crate::data::WorkOrder, bom: &str) -> Value {
    json!({"doctype": "Work Order", "production_item": d.product.code, "bom_no": bom, "qty": w.qty,
           "company": d.company, "planned_batch": w.batch, "description": w.note,
           "source_warehouse": warehouse(d, "US", "Stores"), "wip_warehouse": warehouse(d, "US", "WIP"),
           "fg_warehouse": warehouse(d, "US", "Quarantine"), "planned_start_date": w.start,
           "operations": operation_rows(d)})
}

pub fn quality_procedure() -> Value {
    json!({"doctype": "Quality Procedure", "quality_procedure_name": EM_PROCEDURE})
}

pub fn fictional_note(d: &Data) -> Value {
    json!({"doctype": "Note", "title": "About these records", "public": 1,
           "content": format!("<p>{}</p>", d.fictional)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::data;

    #[test]
    fn records_carry_the_company_and_the_lots_their_status() {
        let d = data();
        assert_eq!(warehouse(d, "EU", "Released"), "EU DC Released - LB");
        let b = batch(d, d.lot("CVX-2598").unwrap());
        assert_eq!(
            (&b["qa_status"], &b["reserved_for"]),
            (&json!("Released"), &json!("EU orders"))
        );
        let so = sales_order(d, d.order("PO-HSA-2026-311").unwrap());
        assert_eq!(so["set_warehouse"], "EU DC Released - LB");
        assert_eq!(so["planned_batch"], "CVX-2598");
        let affected = sales_order(d, d.order("PO-SBMC-4471").unwrap());
        assert_eq!(
            (&affected["transaction_date"], &affected["set_warehouse"]),
            (&json!("2026-10-20"), &json!("US Plant Released - LB"))
        );
        let stock = opening_stock(d);
        assert_eq!(stock.len(), 1 + d.lots.len());
        let posted: Vec<&str> = (stock.iter())
            .map(|s| s["posting_date"].as_str().unwrap())
            .collect();
        let want = [
            "2026-01-02",
            "2026-01-02",
            "2026-01-02",
            "2026-01-29",
            "2026-10-19",
            "2026-11-01",
        ];
        assert_eq!(posted[1..], want);
        assert_eq!(warehouses(d).len(), 8);
        let c = customer(d.customer("Pinecrest Wholesale Drug").unwrap());
        assert_eq!(
            (&c["customer_group"], &c["min_shelf_life_days"]),
            (&json!("Wholesalers"), &json!(270))
        );
    }
}
