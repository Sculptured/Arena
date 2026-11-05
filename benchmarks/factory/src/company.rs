//! The fictional company's ERP records beyond the incident's own: chart, warehouses, items and
//! their prices, the press lines as workstations, the presses as assets with their maintenance,
//! stock, BOMs, and the week's sales and work orders. Each function only builds documents; `seed`
//! writes them. Everything is from the factory example's data and `data/company.json`.
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::date::Day;

pub const ABBR: &str = "RC";
const STAMPED: &str = "Stamped parts";
const SPARES: &str = "MRO spares";
const CUSTOMER_GROUP: &str = "Vehicle makers";
const TERRITORY: &str = "North America";
const WORKSTATION_TYPE: &str = "Stamping press line";
const OPERATION: &str = "Stamping";
const ASSET_CATEGORY: &str = "Stamping presses";
const MAINTENANCE_TEAM: &str = "Ridgeline maintenance";
/// Kg of coil per kg of finished part.
const SCRAP_FACTOR: f64 = 1.25;
/// Spares to keep on the shelf; the failing press's part is below this.
const SPARE_MIN_STOCK: u64 = 1;

/// The records' source: the factory example's data, this benchmark's additions, and today.
pub struct Company<'a> {
    pub data: &'a Value,
    pub extra: &'a Value,
    pub today: Day,
}

/// `<name> - RC`, as ERPNext names a company's accounts and warehouses.
pub fn of_company(name: &str) -> String {
    format!("{name} - {ABBR}")
}

pub fn list<'a>(v: &'a Value, name: &str) -> impl Iterator<Item = &'a Value> {
    v[name].as_array().into_iter().flatten()
}

fn str_of<'a>(v: &'a Value, name: &str) -> &'a str {
    v[name].as_str().unwrap_or_default()
}

impl<'a> Company<'a> {
    pub fn name(&self) -> &'a str {
        str_of(self.data, "company")
    }

    fn line(&self, id: &str) -> Result<&'a Value> {
        list(self.data, "lines")
            .find(|l| l["id"] == id)
            .ok_or_else(|| anyhow!("the data has no line {id}"))
    }

    fn plant(&self, id: &str) -> Result<&'a Value> {
        list(self.data, "plants")
            .find(|p| p["id"] == id)
            .ok_or_else(|| anyhow!("the data has no plant {id}"))
    }

    fn item(&self, code: &str) -> Result<&'a Value> {
        list(self.data, "items")
            .find(|i| i["code"] == code)
            .ok_or_else(|| anyhow!("the data has no item {code}"))
    }

    fn orders(&self) -> impl Iterator<Item = &'a Value> {
        list(self.data, "orders")
    }

    /// `"Plant 2"` of `"Plant 2 (Stamping)"`.
    fn short(plant: &Value) -> &str {
        str_of(plant, "name").split(" (").next().unwrap_or_default()
    }

    /// The warehouse of `plant` with this role (`Stores`, `WIP`, `Finished Goods`).
    fn warehouse(&self, plant: &str, role: &str) -> Result<String> {
        let short = Self::short(self.plant(plant)?);
        Ok(of_company(&format!("{short} {role}")))
    }

    pub fn company_doc(&self) -> Value {
        json!({"doctype": "Company", "company_name": self.name(), "abbr": ABBR,
               "default_currency": self.data["currency"], "country": "United States",
               "create_chart_of_accounts_based_on": "Standard Template",
               "chart_of_accounts": "Standard"})
    }

    /// The calendar years the seed's dates fall in.
    pub fn fiscal_years(&self) -> Vec<Value> {
        let mut years = vec![self.today.plus(-30).year(), self.today.plus(30).year()];
        years.dedup();
        let doc = |y: &i64| {
            json!({"doctype": "Fiscal Year", "year": y.to_string(),
                   "year_start_date": format!("{y}-01-01"), "year_end_date": format!("{y}-12-31")})
        };
        years.iter().map(doc).collect()
    }

    /// Each plant's group, its three warehouses (groups first) and its floor.
    pub fn sites(&self) -> Vec<Value> {
        let mut out = vec![];
        for p in list(self.data, "plants") {
            let short = Self::short(p);
            out.push(
                json!({"doctype": "Warehouse", "warehouse_name": short, "is_group": 1,
                            "company": self.name(),
                            "parent_warehouse": of_company("All Warehouses")}),
            );
            for role in ["Stores", "WIP", "Finished Goods"] {
                out.push(json!({"doctype": "Warehouse", "company": self.name(),
                                "warehouse_name": format!("{short} {role}"),
                                "parent_warehouse": of_company(short)}));
            }
            out.push(json!({"doctype": "Plant Floor", "floor_name": p["name"],
                            "company": self.name(),
                            "warehouse": of_company(str_of(p, "warehouse"))}));
        }
        out
    }

    /// The supplier groups, then the suppliers.
    pub fn suppliers(&self) -> Vec<Value> {
        let mut groups: Vec<&str> = list(self.data, "suppliers")
            .map(|s| str_of(s, "group"))
            .collect();
        groups.sort_unstable();
        groups.dedup();
        let group = |g: &&str| {
            json!({"doctype": "Supplier Group", "supplier_group_name": g,
                   "parent_supplier_group": "All Supplier Groups"})
        };
        let mut out: Vec<Value> = groups.iter().map(group).collect();
        out.extend(list(self.data, "suppliers").map(|s| {
            json!({"doctype": "Supplier", "supplier_name": s["name"], "supplier_group": s["group"],
                   "supplier_type": "Company", "country": "United States",
                   "supplier_details": s["group"]})
        }));
        out
    }

    fn item_suppliers(&self, code: &str) -> Vec<Value> {
        let sold_by = |s: &&Value| list(s, "history").any(|h| h["item"] == code);
        let names = list(self.data, "suppliers").filter(sold_by);
        names.map(|s| json!({"supplier": s["name"]})).collect()
    }

    /// A spare's shelf minimum, in each plant's stores whose presses use it.
    fn reorder_levels(&self, code: &str) -> Result<Vec<Value>> {
        let mut plants: Vec<&str> = vec![];
        for m in list(self.data, "machines").filter(|m| m["spindle_part"] == code) {
            plants.push(str_of(self.line(str_of(m, "line"))?, "plant"));
        }
        plants.sort_unstable();
        plants.dedup();
        let mut out = vec![];
        for p in plants {
            out.push(json!({"warehouse": self.warehouse(p, "Stores")?,
                            "warehouse_reorder_level": SPARE_MIN_STOCK,
                            "warehouse_reorder_qty": SPARE_MIN_STOCK,
                            "material_request_type": "Purchase"}));
        }
        Ok(out)
    }

    fn item_doc(&self, i: &Value) -> Result<Value> {
        let code = str_of(i, "code");
        let group = str_of(i, "group");
        let (sold, spare) = (group == STAMPED, group == SPARES);
        let role = if sold { "Finished Goods" } else { "Stores" };
        let reorder = if spare {
            self.reorder_levels(code)?
        } else {
            vec![]
        };
        Ok(
            json!({"doctype": "Item", "item_code": code, "item_name": i["name"],
                  "description": i["name"], "item_group": group, "stock_uom": i["uom"],
                  "is_stock_item": 1, "include_item_in_manufacturing": 1,
                  "is_sales_item": i32::from(sold), "is_purchase_item": i32::from(!sold),
                  "valuation_rate": if sold { json!(0) } else { i["price"].clone() },
                  "weight_per_unit": i.get("weight_kg").cloned().unwrap_or(json!(0)),
                  "weight_uom": "Kg",
                  "item_defaults": [{"company": self.name(),
                                     "default_warehouse": self.warehouse("P2", role)?}],
                  "supplier_items": self.item_suppliers(code),
                  "reorder_levels": reorder}),
        )
    }

    /// The item groups, some of which a new site already has.
    pub fn item_groups(&self) -> Vec<Value> {
        let mut groups: Vec<&str> = list(self.data, "items")
            .map(|i| str_of(i, "group"))
            .collect();
        groups.sort_unstable();
        groups.dedup();
        let group = |g: &&str| {
            json!({"doctype": "Item Group", "item_group_name": g,
                   "parent_item_group": "All Item Groups"})
        };
        groups.iter().map(group).collect()
    }

    pub fn items(&self) -> Result<Vec<Value>> {
        list(self.data, "items").map(|i| self.item_doc(i)).collect()
    }

    /// Each item's list price, and what each supplier last charged, dated when it did.
    pub fn prices(&self) -> Vec<Value> {
        let listed = list(self.data, "items").map(|i| {
            let selling = i["group"] == STAMPED;
            let price_list = if selling {
                "Standard Selling"
            } else {
                "Standard Buying"
            };
            json!({"doctype": "Item Price", "item_code": i["code"], "price_list": price_list,
                   "price_list_rate": i["price"]})
        });
        let paid = list(self.data, "suppliers").flat_map(|s| {
            list(s, "history").map(|h| {
                let days = h["days_ago"].as_i64().unwrap_or(0);
                json!({"doctype": "Item Price", "item_code": h["item"], "supplier": s["name"],
                       "price_list": "Standard Buying", "price_list_rate": h["rate"],
                       "valid_from": self.today.plus(-days).iso()})
            })
        });
        listed.chain(paid).collect()
    }

    /// A type for them, a workstation per press line (the plant's MES), and the operation.
    pub fn workstations(&self) -> Result<Vec<Value>> {
        let mut out = vec![json!({"doctype": "Workstation Type",
                                  "workstation_type": WORKSTATION_TYPE})];
        for l in list(self.data, "lines") {
            let machines = list(self.data, "machines").filter(|m| m["line"] == l["id"]);
            let described: Vec<&str> = machines.map(|m| str_of(m, "name")).collect();
            let plant = self.plant(str_of(l, "plant"))?;
            out.push(
                json!({"doctype": "Workstation", "workstation_name": l["name"],
                            "workstation_type": WORKSTATION_TYPE,
                            "description": format!("{}: {}", str_of(l, "id"), described.join(", ")),
                            "plant_floor": plant["name"], "production_capacity": 1,
                            "warehouse": of_company(str_of(plant, "warehouse")),
                            "status": "Production"}),
            );
        }
        out.push(json!({"doctype": "Operation", "name": OPERATION,
                        "description": "Stamp the part from coil on a press line"}));
        Ok(out)
    }

    /// The group and territory of the customers, then the customers.
    pub fn customers(&self) -> Vec<Value> {
        let mut out = vec![
            json!({"doctype": "Customer Group", "customer_group_name": CUSTOMER_GROUP,
                   "parent_customer_group": "All Customer Groups"}),
            json!({"doctype": "Territory", "territory_name": TERRITORY,
                   "parent_territory": "All Territories"}),
        ];
        out.extend(list(self.data, "customers").map(|c| {
            json!({"doctype": "Customer", "customer_name": c["name"], "customer_type": "Company",
                   "customer_group": CUSTOMER_GROUP, "territory": TERRITORY,
                   "customer_details": c["terms"]})
        }));
        out
    }

    /// `dana.ruiz@ridgeline.example` for Dana Ruiz.
    fn technician_user(person: &str) -> String {
        format!(
            "{}@ridgeline.example",
            person.to_lowercase().replace(' ', ".")
        )
    }

    /// Users for the technicians, and the maintenance team they are on.
    pub fn team(&self) -> Vec<Value> {
        let techs: Vec<&str> = list(self.data, "technicians")
            .map(|t| str_of(t, "name"))
            .collect();
        let mut out: Vec<Value> = techs
            .iter()
            .map(|&name| {
                let (first, last) = name.split_once(' ').unwrap_or((name, ""));
                json!({"doctype": "User", "email": Self::technician_user(name),
                       "first_name": first, "last_name": last, "user_type": "System User",
                       "send_welcome_email": 0, "enabled": 1,
                       "roles": [{"role": "Maintenance User"}]})
            })
            .collect();
        let members: Vec<Value> = techs
            .iter()
            .map(|name| {
                json!({"team_member": Self::technician_user(name),
                       "maintenance_role": "Maintenance User"})
            })
            .collect();
        out.push(json!({"doctype": "Asset Maintenance Team",
                        "maintenance_team_name": MAINTENANCE_TEAM, "company": self.name(),
                        "maintenance_manager": "Administrator",
                        "maintenance_team_members": members}));
        out
    }

    /// The category, the plants' locations and an item per kind of press.
    pub fn asset_masters(&self) -> Vec<Value> {
        let mut out = vec![json!({"doctype": "Asset Category",
            "asset_category_name": ASSET_CATEGORY, "non_depreciable_category": 1,
            "accounts": [{"company_name": self.name(),
                          "fixed_asset_account": of_company("Plants and Machineries"),
                          "accumulated_depreciation_account": of_company("Accumulated Depreciation"),
                          "depreciation_expense_account": of_company("Depreciation")}]})];
        out.extend(
            list(self.data, "plants")
                .map(|p| json!({"doctype": "Location", "location_name": p["name"]})),
        );
        let mut kinds: Vec<&str> = list(self.data, "machines")
            .map(|m| str_of(m, "kind"))
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        out.extend(kinds.iter().map(|k| {
            json!({"doctype": "Item", "item_code": format!("FA-{}", k.to_uppercase()),
                   "item_name": k.replace('-', " "), "item_group": "All Item Groups",
                   "stock_uom": "Nos", "is_stock_item": 0, "is_fixed_asset": 1,
                   "asset_category": ASSET_CATEGORY})
        }));
        out
    }

    /// The press as an asset that needs maintenance.
    pub fn asset(&self, m: &Value) -> Result<Value> {
        let kind = str_of(m, "kind");
        let bought = &self.extra["press_purchases"][kind];
        let line = self.line(str_of(m, "line"))?;
        let plant = self.plant(str_of(line, "plant"))?;
        Ok(json!({"doctype": "Asset", "asset_name": m["name"],
                  "item_code": format!("FA-{}", kind.to_uppercase()), "company": self.name(),
                  "location": plant["name"], "asset_type": "Existing Asset",
                  "purchase_date": bought["date"], "available_for_use_date": bought["date"],
                  "net_purchase_amount": bought["amount"], "calculate_depreciation": 0,
                  "maintenance_required": 1}))
    }

    /// The task of `extra`'s list that a line of a press's history is about.
    fn task_of(&self, work: &str) -> Result<&'a Value> {
        let text = work.to_lowercase();
        list(self.extra, "maintenance_tasks")
            .find(|t| list(t, "marks").any(|m| m.as_str().is_some_and(|m| text.contains(m))))
            .ok_or_else(|| anyhow!("no maintenance task for {work:?}"))
    }

    /// `asset`'s maintenance plan: one task per kind of work in the press's history, each started
    /// on the first time it was done.
    pub fn maintenance(&self, m: &Value, asset: &str) -> Result<Value> {
        let mut tasks: Vec<Value> = vec![];
        let mut history: Vec<&Value> = list(m, "history").collect();
        history.sort_by_key(|h| str_of(h, "date"));
        for h in history {
            let task = self.task_of(str_of(h, "work"))?;
            if tasks.iter().any(|t| t["maintenance_task"] == task["task"]) {
                continue;
            }
            tasks.push(
                json!({"maintenance_task": task["task"], "periodicity": task["periodicity"],
                              "maintenance_type": "Preventive Maintenance",
                              "maintenance_status": "Planned", "start_date": h["date"],
                              "assign_to": Self::technician_user(str_of(h, "by"))}),
            );
        }
        Ok(
            json!({"doctype": "Asset Maintenance", "asset_name": asset, "company": self.name(),
                  "maintenance_team": MAINTENANCE_TEAM, "asset_maintenance_tasks": tasks}),
        )
    }

    /// A completed log for each line of the press's history; `plan` is its saved plan.
    pub fn maintenance_logs(&self, m: &Value, plan: &Value) -> Result<Vec<Value>> {
        let mut out = vec![];
        for h in list(m, "history") {
            let work = str_of(h, "work");
            let task = self.task_of(work)?;
            let row = list(plan, "asset_maintenance_tasks")
                .find(|r| r["maintenance_task"] == task["task"])
                .with_context(|| format!("no plan row for {}", task["task"]))?;
            let done = format!("{work}. Done by {}.", str_of(h, "by"));
            out.push(json!({"doctype": "Asset Maintenance Log",
                            "asset_maintenance": plan["name"], "task": row["name"],
                            "maintenance_status": "Completed", "completion_date": h["date"],
                            "actions_performed": done}));
        }
        Ok(out)
    }

    /// The opening stock, 30 days back. The failing press's part has none on purpose.
    pub fn opening_stock(&self) -> Result<Value> {
        let mut rows = vec![];
        for s in list(self.extra, "opening_stock") {
            let code = str_of(s, "item");
            rows.push(json!({"item_code": code, "qty": s["qty"],
                             "t_warehouse": self.warehouse(str_of(s, "plant"), "Stores")?,
                             "basic_rate": self.item(code)?["price"]}));
        }
        Ok(
            json!({"doctype": "Stock Entry", "stock_entry_type": "Material Receipt",
                  "purpose": "Material Receipt", "company": self.name(), "set_posting_time": 1,
                  "posting_date": self.today.plus(-30).iso(), "remarks": "Opening stock",
                  "items": rows}),
        )
    }

    /// The minutes one part takes on `line`.
    fn minutes_each(line: &Value) -> f64 {
        let rate = line["rate_per_hour"].as_f64().unwrap_or(60.0);
        (60_000.0 / rate).round() / 1000.0
    }

    fn operation_row(line: &Value) -> Value {
        json!({"operation": OPERATION, "workstation": line["name"],
               "time_in_mins": Self::minutes_each(line)})
    }

    /// The BOM of each stamped part: its coil, and a stamping operation on the first line that
    /// makes it in this week's orders.
    pub fn boms(&self) -> Result<Vec<Value>> {
        let stamped = list(self.data, "items").filter(|i| i["group"] == STAMPED);
        let mut out = vec![];
        for i in stamped {
            let first = self.orders().find(|o| o["item"] == i["code"]);
            let line = first.map_or("LINE-A", |o| str_of(o, "line"));
            let coil = i["weight_kg"].as_f64().unwrap_or(0.0) * SCRAP_FACTOR;
            out.push(
                json!({"doctype": "BOM", "item": i["code"], "company": self.name(),
                            "quantity": 1, "is_active": 1, "is_default": 1,
                            "rm_cost_as_per": "Valuation Rate", "with_operations": 1,
                            "operations": [Self::operation_row(self.line(line)?)],
                            "items": [{"item_code": "STL-DP780",
                                       "qty": (coil * 1000.0).round() / 1000.0}]}),
            );
        }
        Ok(out)
    }

    /// The date an order is due.
    pub fn due(&self, order: &Value) -> Day {
        self.today.plus(order["due_in_days"].as_i64().unwrap_or(0))
    }

    /// The customer's order; its `po_no` is the order's id in the plant's list.
    pub fn sales_order(&self, o: &Value) -> Result<Value> {
        let code = str_of(o, "item");
        let line = self.line(str_of(o, "line"))?;
        let plant = str_of(line, "plant");
        let store = self.warehouse(plant, "Finished Goods")?;
        let due = self.due(o).iso();
        Ok(
            json!({"doctype": "Sales Order", "customer": o["customer"], "company": self.name(),
                  "order_type": "Sales", "transaction_date": self.today.plus(-14).iso(),
                  "delivery_date": due, "po_no": o["id"], "set_warehouse": store,
                  "selling_price_list": "Standard Selling",
                  "items": [{"item_code": code, "qty": o["qty"], "delivery_date": due,
                             "rate": self.item(code)?["price"], "warehouse": store}]}),
        )
    }

    /// The order's work on its line, whose job cards the MES runs.
    pub fn work_order(&self, o: &Value, sales_order: &str, bom: &str) -> Result<Value> {
        let line = self.line(str_of(o, "line"))?;
        let plant = str_of(line, "plant");
        Ok(
            json!({"doctype": "Work Order", "production_item": o["item"], "bom_no": bom,
                  "qty": o["qty"], "company": self.name(), "sales_order": sales_order,
                  "source_warehouse": self.warehouse(plant, "Stores")?,
                  "wip_warehouse": self.warehouse(plant, "WIP")?,
                  "fg_warehouse": self.warehouse(plant, "Finished Goods")?,
                  "planned_start_date": format!("{} 06:00:00", self.today.iso()),
                  "expected_delivery_date": self.due(o).iso(),
                  "operations": [Self::operation_row(line)]}),
        )
    }

    /// The spare parts on the shelf, so that a reader need not sum the stock ledger.
    pub fn stock_note(&self) -> Value {
        let on_hand = |code: &str| -> u64 {
            let rows = list(self.extra, "opening_stock").filter(|s| s["item"] == code);
            rows.map(|s| s["qty"].as_u64().unwrap_or(0)).sum()
        };
        let spares = list(self.data, "items").filter(|i| i["group"] == SPARES);
        let rows: Vec<String> = spares
            .map(|i| {
                let code = str_of(i, "code");
                format!(
                    "<li>{code}: {} on hand, minimum {SPARE_MIN_STOCK}</li>",
                    on_hand(code)
                )
            })
            .collect();
        json!({"doctype": "Note", "title": "Spare parts on hand", "public": 1,
               "content": format!("<ul>{}</ul>", rows.join(""))})
    }

    pub fn fictional_note(&self) -> Value {
        json!({"doctype": "Note", "title": "About these records", "public": 1,
               "content": format!("<p>{}</p>", str_of(self.extra, "fictional"))})
    }
}
