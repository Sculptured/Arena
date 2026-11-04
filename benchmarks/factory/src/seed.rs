//! The ERP's starting records, from the factory example's data: the spare parts, suppliers and
//! customers, the press's service manual and the orders on its line, written through its REST API.
//! The hub and the plant start from the example's own data.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::facts::{machine, MACHINE};

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

/// The orders on the press's line, for the team to see who a stopped line affects.
fn orders_note(data: &Value) -> Result<Value> {
    let line = &machine(data)?["line"];
    let orders = data["orders"].as_array().into_iter().flatten();
    let rows: Vec<String> = orders
        .filter(|o| &o["line"] == line)
        .map(|o| {
            format!(
                "<li>{} {} {} x {}, due in {} days</li>",
                o["id"], o["customer"], o["qty"], o["item"], o["due_in_days"]
            )
        })
        .collect();
    Ok(
        json!({"doctype": "Note", "title": format!("Orders on {MACHINE}'s line"), "public": 1,
              "content": format!("<ul>{}</ul>", rows.join(""))}),
    )
}

fn spares(data: &Value) -> Vec<Value> {
    let items = data["items"].as_array().into_iter().flatten();
    items
        .filter(|i| i["code"].as_str().is_some_and(|c| c.starts_with("SP-")))
        .map(|i| {
            json!({"doctype": "Item", "item_code": i["code"], "item_name": i["name"],
                        "item_group": "Products", "stock_uom": "Nos", "is_stock_item": 0})
        })
        .collect()
}

fn suppliers(data: &Value) -> Vec<Value> {
    let list = data["suppliers"].as_array().into_iter().flatten();
    list.map(|s| {
        let group = s["group"].as_str().unwrap_or_default();
        json!({"doctype": "Supplier", "supplier_name": s["name"], "supplier_details": group})
    })
    .collect()
}

fn customers(data: &Value) -> Vec<Value> {
    let list = data["customers"].as_array().into_iter().flatten();
    list.map(|c| {
        json!({"doctype": "Customer", "customer_name": c["name"],
                        "customer_details": c["terms"]})
    })
    .collect()
}

/// Every starting record, in the order they are written.
pub fn records(data: &Value) -> Result<Vec<Value>> {
    let mut out = spares(data);
    out.extend(suppliers(data));
    out.extend(customers(data));
    out.push(manual_note(data)?);
    out.push(orders_note(data)?);
    Ok(out)
}

fn field<'a>(erp: &'a Value, name: &str) -> Result<&'a str> {
    erp[name]
        .as_str()
        .ok_or_else(|| anyhow!("the erp part's connect info has no {name}"))
}

/// Writes the records through the instance's REST API (`erp` is its `connect` info).
pub async fn seed(erp: &Value, data: &Value) -> Result<()> {
    let auth = format!(
        "token {}:{}",
        field(erp, "api_key")?,
        field(erp, "api_secret")?
    );
    let (url, site) = (field(erp, "url")?, field(erp, "site")?);
    let client = reqwest::Client::new();
    for doc in records(data)? {
        let doctype = doc["doctype"].as_str().unwrap_or_default();
        let at = format!("{}/api/resource/{doctype}", url.trim_end_matches('/'));
        let req = client
            .post(at)
            .header("Host", site)
            .header("Authorization", &auth);
        let answer = req.json(&doc).send().await?;
        let status = answer.status();
        if !status.is_success() {
            bail!("writing {doc}: {status}: {}", answer.text().await?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        json!({
            "machines": [{"id": "PR-A1", "line": "L", "kind": "press", "spindle_part": "P-1"}],
            "main_drives": {"press": {"shaft_rpm": 300, "bearing": {"designation": "B"},
                                      "defect_orders": {"ftf": 0.4, "bpfo": 9.596,
                                                        "bpfi": 12.4, "bsf": 3.7}}},
            "items": [{"code": "SP-1", "name": "Spindle"}, {"code": "RC-1", "name": "Bracket"}],
            "suppliers": [{"name": "Good", "group": "Approved"}],
            "customers": [{"name": "C", "terms": "Net 30"}],
            "orders": [{"id": "A-1", "customer": "C", "line": "L"}, {"id": "Z", "line": "M"}],
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
    fn only_spares_and_the_presss_line_are_seeded() {
        let all = records(&data()).unwrap();
        let of = |t: &str| all.iter().filter(|d| d["doctype"] == t).count();
        assert_eq!(
            (of("Item"), of("Supplier"), of("Customer"), of("Note")),
            (1, 1, 1, 2)
        );
        let orders = all.last().unwrap()["content"].as_str().unwrap();
        assert!(orders.contains("A-1") && !orders.contains("Z"), "{orders}");
    }
}
