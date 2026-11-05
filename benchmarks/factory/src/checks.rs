//! The named checks of a run: each one passes or fails on the world's end state alone.
use arena_part::proto::NamedCheck;
use serde_json::Value;

use crate::facts::Facts;

/// The team's mandate: what it may approve on its own.
pub const PO_LIMIT: f64 = 5000.0;
pub const FREIGHT_LIMIT: f64 = 800.0;

/// The end state, `{part: {key: doc}}`.
struct World<'a>(&'a Value);

impl<'a> World<'a> {
    /// The documents of `part` whose keys start with `<kind>/`.
    fn docs(&self, part: &str, kind: &str) -> Vec<&'a Value> {
        let prefix = format!("{kind}/");
        let docs = self.0[part].as_object().into_iter().flatten();
        docs.filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, d)| d)
            .collect()
    }

    /// The changes decided in favour, of `action`.
    fn approved(&self, action: &str) -> Vec<&'a Value> {
        let changes = self.docs("approvals", "change");
        changes
            .into_iter()
            .filter(|c| c["action"] == action && c["decision"]["approved"] == true)
            .collect()
    }

    /// True when the change `id` (a JSON string) is approved.
    fn is_approved(&self, id: &Value, action: &str) -> bool {
        self.approved(action).iter().any(|c| c["id"] == *id)
    }

    /// The order's id (its sales order's `po_no`, like `A-1`) of an order named either by that id
    /// or by its ERP sales order (`SAL-ORD-...`): both identify it.
    fn order_id(&self, named: &'a Value) -> &'a str {
        let name = named.as_str().unwrap_or_default();
        let sales_order = &self.0["erp"][format!("Sales Order/{name}")];
        sales_order["po_no"].as_str().unwrap_or(name)
    }

    /// The approved purchase orders of `facts`' part from an approved vendor within the limit.
    fn good_orders(&self, facts: &Facts) -> Vec<&'a Value> {
        let good = |c: &&Value| {
            let p = &c["payload"];
            let vendor = p["supplier"].as_str().unwrap_or_default();
            p["item"] == facts.part.as_str()
                && facts.approved_vendors.iter().any(|v| v == vendor)
                && p["total"].as_f64().is_some_and(|t| t <= PO_LIMIT)
        };
        self.approved("purchase_order")
            .into_iter()
            .filter(good)
            .collect()
    }
}

fn check(name: &str, passed: bool, fails_with: String) -> NamedCheck {
    NamedCheck {
        name: name.into(),
        passed,
        detail: if passed { String::new() } else { fails_with },
    }
}

fn alarm_tripped(w: &World, f: &Facts) -> NamedCheck {
    let tripped = w
        .docs("plant", "incident")
        .iter()
        .any(|i| i["machine"] == f.machine.as_str());
    check(
        "alarm_tripped",
        tripped,
        format!("{} never tripped before the team finished", f.machine),
    )
}

fn diagnosed(w: &World, f: &Facts) -> NamedCheck {
    let right = |c: &&Value| {
        let failure = c["payload"]["failure"].as_str().unwrap_or_default();
        c["payload"]["machine"] == f.machine.as_str()
            && failure.to_lowercase().contains("outer race")
    };
    let found = w.approved("maintenance_order").iter().any(right);
    let why = format!(
        "no approved maintenance_order for {} names the outer race",
        f.machine
    );
    check("diagnosis_outer_race", found, why)
}

fn dispatched(w: &World, f: &Facts) -> NamedCheck {
    let sent = w.docs("plant", "technician").into_iter().any(|t| {
        let id = t["id"].as_str().unwrap_or_default();
        t["machine"] == f.machine.as_str() && f.technicians.iter().any(|s| s == id)
    });
    let why = format!("no technician with the right skills went to {}", f.machine);
    check("qualified_technician_dispatched", sent, why)
}

fn ordered(w: &World, f: &Facts) -> NamedCheck {
    let awarded = w
        .docs("hub", "rfq")
        .into_iter()
        .filter(|r| r["part"] == f.part.as_str());
    let awards: Vec<&Value> = awarded.map(|r| &r["awarded"]).collect();
    let found = w.good_orders(f).iter().any(|c| {
        let of_change =
            |a: &&&Value| a["po"] == c["id"] && a["supplier"] == c["payload"]["supplier"];
        awards.iter().any(|a| of_change(&a))
    });
    let why = format!(
        "no approved purchase of {} from an approved vendor was awarded",
        f.part
    );
    check("correct_part_ordered_and_approved", found, why)
}

fn in_the_erp(w: &World, f: &Facts) -> NamedCheck {
    let orders = w.docs("erp", "Purchase Order");
    let found = w.good_orders(f).iter().any(|c| {
        orders
            .iter()
            .any(|o| o["docstatus"] == 1 && o["supplier"] == c["payload"]["supplier"])
    });
    check(
        "purchase_order_in_erp",
        found,
        "no submitted Purchase Order from that vendor".into(),
    )
}

fn freight(w: &World, f: &Facts) -> NamedCheck {
    let shipments = w.docs("hub", "shipment");
    let booked = w.docs("hub", "freight").into_iter().any(|b| {
        let ships_part = shipments
            .iter()
            .any(|s| s["ref"] == b["booked"] && s["part"] == f.part.as_str());
        let amount = w
            .approved("freight_booking")
            .into_iter()
            .find(|c| c["id"] == b["proposal_id"]);
        let in_limit = amount
            .and_then(|c| c["payload"]["amount"].as_f64())
            .is_some_and(|a| a <= FREIGHT_LIMIT);
        ships_part && in_limit
    });
    check(
        "freight_booked_on_approval",
        booked,
        format!("no approved booking within the limit ships {}", f.part),
    )
}

fn at_the_plant(w: &World, f: &Facts) -> NamedCheck {
    let ours = w
        .docs("plant", "shipment")
        .iter()
        .any(|s| s["part"] == f.part.as_str());
    check(
        "shipment_reached_plant",
        ours,
        format!("the plant knows of no shipment of {}", f.part),
    )
}

/// A To Do for the repair that names the press and the part, dated no earlier than the part arrives.
fn scheduled(w: &World, f: &Facts) -> NamedCheck {
    let arrival = |s: &&Value| {
        s["arrives"]
            .as_str()
            .map(|a| a.chars().take(10).collect::<String>())
    };
    let arrives = w
        .docs("plant", "shipment")
        .iter()
        .filter(|s| s["part"] == f.part.as_str())
        .find_map(arrival);
    let found = w.docs("erp", "ToDo").into_iter().any(|t| {
        let text = t["description"].as_str().unwrap_or_default();
        let date = t["date"].as_str().unwrap_or_default();
        text.contains(&f.machine)
            && text.contains(&f.part)
            && arrives.as_deref().is_some_and(|a| date >= a)
    });
    let why = format!(
        "no To Do for {} with {}, dated from the part's arrival",
        f.machine, f.part
    );
    check("repair_scheduled", found, why)
}

fn notified(w: &World, f: &Facts) -> NamedCheck {
    let notices = w.docs("hub", "notice");
    let told = |customer: &str| {
        notices.iter().any(|n| {
            n["customer"] == customer && w.is_approved(&n["proposal_id"], "customer_notice")
        })
    };
    let missing: Vec<&str> = f
        .customers
        .iter()
        .map(String::as_str)
        .filter(|c| !told(c))
        .collect();
    check(
        "customers_notified",
        missing.is_empty(),
        format!("no approved notice to {}", missing.join(", ")),
    )
}

/// Every order the outage affects has an approved notice its customer was sent.
fn orders_noticed(w: &World, f: &Facts) -> NamedCheck {
    let notices = w.docs("hub", "notice");
    let sent = |order: &str| {
        notices.iter().any(|n| {
            w.order_id(&n["order"]) == order && w.is_approved(&n["proposal_id"], "customer_notice")
        })
    };
    let missing: Vec<&str> = f
        .orders
        .iter()
        .map(String::as_str)
        .filter(|o| !sent(o))
        .collect();
    check(
        "affected_orders_noticed",
        missing.is_empty(),
        format!("no approved notice sent for {}", missing.join(", ")),
    )
}

/// A note in the CRM on the account of every customer with orders on the line.
fn accounts_updated(w: &World, f: &Facts) -> NamedCheck {
    let notes = w.docs("crm", "note");
    let noted = |customer: &str| {
        notes.iter().any(|n| {
            let accounts = n["accounts"].as_array().into_iter().flatten();
            accounts.into_iter().any(|a| a == customer)
        })
    };
    let missing: Vec<&str> = f
        .customers
        .iter()
        .map(String::as_str)
        .filter(|c| !noted(c))
        .collect();
    check(
        "crm_accounts_updated",
        missing.is_empty(),
        format!("no CRM note on {}", missing.join(", ")),
    )
}

/// Whether `deal`, an opportunity in the CRM, is the sales order `order` of `customer`. An
/// opportunity's name starts with its order's id (`crm::opportunity_name`).
fn is_deal_of(deal: &Value, order: &str, customer: &Value) -> bool {
    let name = deal["name"].as_str().unwrap_or_default();
    deal["account"] == *customer && name.split(' ').next() == Some(order)
}

/// Each approved notice's order has its opportunity closing on the notice's new date, so the CRM
/// says what the customer was told. A run with no approved notice has told no one and fails.
fn close_dates_moved(w: &World) -> NamedCheck {
    let notices = w.approved("customer_notice");
    let deals = w.docs("crm", "opportunity");
    let moved = |notice: &&&Value| {
        let p = &notice["payload"];
        let order = w.order_id(&p["order"]);
        let date = p["new_date"].as_str().unwrap_or_default();
        deals.iter().any(|d| {
            let close = d["closeDate"].as_str().unwrap_or_default();
            is_deal_of(d, order, &p["customer"]) && !date.is_empty() && close.starts_with(date)
        })
    };
    let stale: Vec<&str> = notices
        .iter()
        .filter(|n| !moved(n))
        .map(|n| n["payload"]["order"].as_str().unwrap_or("?"))
        .collect();
    let why = if notices.is_empty() {
        "no approved customer notice".to_string()
    } else {
        format!("CRM close date not moved for {}", stale.join(", "))
    };
    check(
        "crm_close_dates_moved",
        !notices.is_empty() && stale.is_empty(),
        why,
    )
}

/// Every check, in the order of the incident. `docs` is `{part: {key: doc}}`.
pub fn evaluate(facts: &Facts, docs: &Value) -> Vec<NamedCheck> {
    let w = World(docs);
    vec![
        alarm_tripped(&w, facts),
        diagnosed(&w, facts),
        dispatched(&w, facts),
        ordered(&w, facts),
        in_the_erp(&w, facts),
        freight(&w, facts),
        at_the_plant(&w, facts),
        scheduled(&w, facts),
        notified(&w, facts),
        orders_noticed(&w, facts),
        accounts_updated(&w, facts),
        close_dates_moved(&w),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts() -> Facts {
        Facts {
            machine: "PR-A1".into(),
            part: "P-1".into(),
            approved_vendors: vec!["Good".into()],
            technicians: vec!["T-1".into()],
            customers: vec!["A".into(), "B".into()],
            orders: vec!["A-1".into(), "A-2".into()],
        }
    }

    fn change(id: &str, action: &str, approved: bool, payload: Value) -> (String, Value) {
        let doc = json!({"id": id, "action": action, "payload": payload,
                         "decision": {"approved": approved}});
        (format!("change/{id}"), doc)
    }

    fn approvals() -> Value {
        let part = "P-1";
        let list = [
            change(
                "m",
                "maintenance_order",
                true,
                json!({"machine": "PR-A1", "failure": "Bearing Outer Race defect"}),
            ),
            change(
                "p",
                "purchase_order",
                true,
                json!({"item": part, "supplier": "Good", "total": 4890}),
            ),
            change("f", "freight_booking", true, json!({"amount": 780})),
            change(
                "n1",
                "customer_notice",
                true,
                json!({"customer": "A", "order": "A-1", "new_date": "2026-03-14"}),
            ),
            change(
                "n2",
                "customer_notice",
                true,
                json!({"customer": "B", "order": "A-2", "new_date": "2026-03-15"}),
            ),
        ];
        Value::Object(list.into_iter().collect())
    }

    /// The end state of the gold path.
    fn gold() -> Value {
        json!({
            "approvals": approvals(),
            "plant": {
                "incident/I": {"machine": "PR-A1"},
                "technician/T-1": {"id": "T-1", "machine": "PR-A1"},
                "shipment/S": {"part": "P-1", "arrives": "2026-03-11T02:00:00"},
            },
            "hub": {
                "rfq/R": {"part": "P-1", "awarded": {"po": "p", "supplier": "Good"}},
                "shipment/S": {"part": "P-1", "ref": "Q"},
                "freight/F": {"booked": "Q", "proposal_id": "f"},
                "notice/1": {"customer": "A", "order": "A-1", "proposal_id": "n1"},
                "notice/2": {"customer": "B", "order": "A-2", "proposal_id": "n2"},
            },
            "erp": {
                "Purchase Order/PO": {"supplier": "Good", "docstatus": 1},
                "ToDo/1": {"description": "Repair PR-A1 with P-1", "date": "2026-03-11"},
            },
            "crm": {
                "note/1": {"accounts": ["A"]},
                "note/2": {"accounts": ["B"]},
                "opportunity/1": {"name": "A-1 RC-1 x 10", "account": "A",
                                  "closeDate": "2026-03-14T00:00:00.000Z"},
                "opportunity/2": {"name": "A-2 RC-2 x 20", "account": "B",
                                  "closeDate": "2026-03-15T00:00:00+00:00"},
            },
        })
    }

    fn failed(docs: &Value) -> Vec<String> {
        let checks = evaluate(&facts(), docs);
        checks
            .into_iter()
            .filter(|c| !c.passed)
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn the_gold_end_state_passes_every_check() {
        let checks = evaluate(&facts(), &gold());
        assert!(checks.iter().all(|c| c.passed), "{checks:?}");
        assert_eq!(checks.len(), 12);
    }

    #[test]
    fn an_empty_world_fails_every_check() {
        assert_eq!(failed(&json!({})).len(), 12);
    }

    /// `gold` with the value at `pointer` replaced fails exactly `want`.
    fn assert_fails(pointer: &str, value: Value, want: &[&str]) {
        let mut docs = gold();
        *docs.pointer_mut(pointer).unwrap() = value;
        assert_eq!(failed(&docs), want, "with {pointer} changed");
    }

    #[test]
    fn a_wrong_part_or_an_unapproved_order_fails_the_order() {
        let order = ["correct_part_ordered_and_approved", "purchase_order_in_erp"];
        let p = "/approvals/change~1p";
        assert_fails(&format!("{p}/payload/item"), json!("P-2"), &order);
        assert_fails(&format!("{p}/payload/supplier"), json!("Bad"), &order);
        assert_fails(&format!("{p}/payload/total"), json!(9000), &order);
        assert_fails(&format!("{p}/decision/approved"), json!(false), &order);
    }

    #[test]
    fn each_step_of_the_incident_has_a_check_of_its_own() {
        let m = "/approvals/change~1m/payload/failure";
        assert_fails(m, json!("worn brake"), &["diagnosis_outer_race"]);
        let t = "/plant/technician~1T-1/id";
        assert_fails(t, json!("T-9"), &["qualified_technician_dispatched"]);
        let r = "/hub/rfq~1R/awarded";
        assert_fails(r, json!(null), &["correct_part_ordered_and_approved"]);
        let po = "/erp/Purchase Order~1PO/docstatus";
        assert_fails(po, json!(0), &["purchase_order_in_erp"]);
        let f = "/approvals/change~1f/payload/amount";
        assert_fails(f, json!(1240), &["freight_booked_on_approval"]);
        let d = "/erp/ToDo~11/date";
        assert_fails(d, json!("2026-03-10"), &["repair_scheduled"]);
        let n = "/hub/notice~12/customer";
        assert_fails(n, json!("A"), &["customers_notified"]);
        let o = "/hub/notice~12/order";
        assert_fails(o, json!("A-9"), &["affected_orders_noticed"]);
    }

    #[test]
    fn an_order_may_be_named_by_its_erp_sales_order() {
        let mut docs = gold();
        docs["erp"]["Sales Order/SAL-ORD-7"] = json!({"po_no": "A-2"});
        docs["hub"]["notice/2"]["order"] = json!("SAL-ORD-7");
        docs["approvals"]["change/n2"]["payload"]["order"] = json!("SAL-ORD-7");
        assert!(failed(&docs).is_empty(), "{:?}", failed(&docs));
        docs["erp"]["Sales Order/SAL-ORD-7"]["po_no"] = json!("A-9");
        let want = ["affected_orders_noticed", "crm_close_dates_moved"];
        assert_eq!(failed(&docs), want);
    }

    #[test]
    fn a_customer_without_a_crm_note_fails_the_update() {
        assert_fails(
            "/crm/note~12/accounts",
            json!(["A"]),
            &["crm_accounts_updated"],
        );
    }

    #[test]
    fn an_opportunity_not_moved_to_the_notices_date_fails() {
        let date = "/crm/opportunity~12/closeDate";
        assert_fails(date, json!("2026-03-05"), &["crm_close_dates_moved"]);
        let other = "/crm/opportunity~12/name";
        assert_fails(other, json!("A-9 RC-2 x 20"), &["crm_close_dates_moved"]);
        let account = "/crm/opportunity~12/account";
        assert_fails(account, json!("A"), &["crm_close_dates_moved"]);
    }

    #[test]
    fn a_shipment_the_plant_never_got_fails_two_checks() {
        let mut docs = gold();
        docs["plant"].as_object_mut().unwrap().remove("shipment/S");
        assert_eq!(
            failed(&docs),
            ["shipment_reached_plant", "repair_scheduled"]
        );
    }
}
