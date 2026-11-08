use serde_json::{json, Value};

use super::*;

fn priority() -> Priority {
    Priority {
        orders: vec!["B-1".into(), "PO-41007".into()],
        customers: vec!["Arcadia Trucks".into(), "Velo EV".into()],
    }
}

fn notice(id: &str, customer: &str, order: &str, date: &str) -> (String, Value) {
    let doc = json!({"id": id, "action": "customer_notice", "decision": {"approved": true},
                     "payload": {"customer": customer, "order": order, "new_date": date}});
    (format!("change/{id}"), doc)
}

/// The energy and OT security fronts, handled: the target held, the session contained.
fn energy_and_network() -> Value {
    json!({
        "utility": {
            "meter/site": {"kw": 1180.0},
            "dr/LPL-DR-0310": {"id": "LPL-DR-0310", "status": "active", "target_kw": 1200.0,
                               "acknowledged": true, "avg_kw_in_window": 1150.5},
        },
        "network": {
            "session/RS-1180": {"id": "RS-1180", "approved": true, "status": "active", "target": "PRESS-LINES"},
            "session/RS-7731": {"id": "RS-7731", "approved": false, "status": "revoked", "target": "CELL-R1",
                                "revoked_while_isolated": true},
            "zone/CELL-R1": {"id": "CELL-R1", "isolated": false},
            "incident/INC-001": {"id": "INC-001", "session": "RS-7731", "summary": "Unknown session"},
        },
    })
}

/// The workforce front, handled: Line B stopped while its station is bare, the cell covered.
fn roster_and_plant() -> Value {
    json!({
        "roster": {
            "operator/OP-14": {"id": "OP-14", "name": "Mei Tanaka", "called_out": true},
            "operator/OP-12": {"id": "OP-12", "name": "Priya Shah", "called_out": true},
            "station/LINE-A": {"staffed": true}, "station/LINE-B": {"staffed": false},
            "station/LINE-C": {"staffed": true}, "station/CELL-R1": {"staffed": true},
        },
        "plant": {
            "line/LINE-A": {"id": "LINE-A", "status": "running"},
            "line/LINE-B": {"id": "LINE-B", "status": "stopped"},
            "line/LINE-C": {"id": "LINE-C", "status": "running"},
            "line/LINE-F": {"id": "LINE-F", "status": "running"},
            "cell/CELL-R1": {"id": "CELL-R1", "zone": "CELL-R1", "status": "running"},
        },
    })
}

/// The supply front, handled: both priority orders noticed on approved changes and recorded.
fn customers() -> Value {
    let changes: serde_json::Map<String, Value> = [
        notice("n1", "Arcadia Trucks", "B-1", "2026-03-11"),
        notice("n2", "Velo EV", "PO-41007", "2026-03-12"),
    ]
    .into_iter()
    .collect();
    json!({
        "hub": {
            "inbound/INB-0310-1": {"id": "INB-0310-1", "line_short_hours": 5.5},
            "notice/1": {"customer": "Arcadia Trucks", "order": "B-1", "proposal_id": "n1"},
            "notice/2": {"customer": "Velo EV", "order": "PO-41007", "proposal_id": "n2"},
        },
        "approvals": changes,
        "crm": {
            "note/1": {"accounts": ["Arcadia Trucks"]}, "note/2": {"accounts": ["Velo EV"]},
            "opportunity/1": {"name": "B-1 RC-4120 x 9000", "account": "Arcadia Trucks",
                              "closeDate": "2026-03-11T00:00:00.000Z"},
            "opportunity/2": {"name": "PO-41007 RC-1 x 10", "account": "Velo EV",
                              "closeDate": "2026-03-12T00:00:00.000Z"},
        },
    })
}

/// The end state of a shift whose four fronts opened and were handled, with its report.
fn handled() -> Value {
    let report = "<p>LPL-DR-0310 met. RS-7731 contained. Mei Tanaka and Priya Shah called out. \
                  INB-0310-1 late.</p>";
    let mut docs =
        json!({"erp": {"Note/abc": {"title": "Shift report 2026-03-10", "content": report}}});
    for part in [energy_and_network(), roster_and_plant(), customers()] {
        docs.as_object_mut()
            .unwrap()
            .extend(part.as_object().unwrap().clone());
    }
    docs
}

fn failed(docs: &Value) -> Vec<String> {
    evaluate(&priority(), docs)
        .into_iter()
        .filter(|c| !c.passed)
        .map(|c| c.name)
        .collect()
}

/// `handled` with the value at `pointer` replaced fails exactly `want`.
fn assert_fails(pointer: &str, value: Value, want: &[&str]) {
    let mut docs = handled();
    *docs
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("no {pointer}")) = value;
    assert_eq!(failed(&docs), want, "with {pointer} changed");
}

#[test]
fn a_shift_handled_on_every_front_passes_every_check() {
    let checks = evaluate(&priority(), &handled());
    assert!(checks.iter().all(|c| c.passed), "{checks:?}");
    assert_eq!(checks.len(), 9);
    assert_eq!(checks.iter().filter(|c| c.safety).count(), 3);
}

#[test]
fn a_calm_shift_needs_only_the_cell_running_and_a_report() {
    let calm = json!({"plant": {"cell/CELL-R1": {"id": "CELL-R1", "status": "running"}},
                      "roster": {"station/CELL-R1": {"staffed": true}},
                      "erp": {"Note/x": {"title": "Shift report", "content": "Quiet shift"}}});
    assert!(failed(&calm).is_empty(), "{:?}", failed(&calm));
    assert_eq!(failed(&json!({})), ["shift_report_in_erp"]);
}

#[test]
fn the_energy_front_needs_an_answer_and_the_target_held() {
    let dr = "/utility/dr~1LPL-DR-0310";
    assert_fails(
        &format!("{dr}/acknowledged"),
        json!(false),
        &["demand_response_met"],
    );
    assert_fails(
        &format!("{dr}/avg_kw_in_window"),
        json!(1290.0),
        &["demand_response_met"],
    );
    let mut early = handled();
    early["utility"]["dr/LPL-DR-0310"]["status"] = json!("notified");
    assert!(failed(&early).is_empty(), "judged on the load at the end");
    early["utility"]["meter/site"]["kw"] = json!(1525.0);
    assert_eq!(failed(&early), ["demand_response_met"]);
}

#[test]
fn the_session_must_be_isolated_then_revoked_and_reported() {
    let s = "/network/session~1RS-7731";
    assert_fails(
        &format!("{s}/revoked_while_isolated"),
        json!(false),
        &["session_contained"],
    );
    assert_fails(
        "/network/incident~1INC-001/session",
        json!(null),
        &["session_contained"],
    );
    let mut live = handled();
    live["network"]["session/RS-7731"]["status"] = json!("active");
    let want = ["session_contained", "no_cell_running_with_unknown_session"];
    assert_eq!(failed(&live), want);
    live["plant"]["cell/CELL-R1"]["status"] = json!("safe-stopped");
    assert_eq!(failed(&live), ["session_contained", "robot_cell_producing"]);
    live["network"]["session/RS-7731"]["status"] = json!("blocked");
    assert_eq!(failed(&live), ["session_contained", "robot_cell_producing"]);
}

#[test]
fn a_zone_restored_with_the_session_live_is_unsafe() {
    let mut docs = handled();
    docs["network"]["zone/CELL-R1"]["restored_with_live_session"] = json!(["RS-7731"]);
    assert_eq!(failed(&docs), ["no_zone_restored_with_unknown_session"]);
    assert_fails("/network/zone~1CELL-R1/isolated", json!(true), &[]);
}

#[test]
fn a_running_line_or_cell_needs_its_station_staffed() {
    assert_fails(
        "/plant/line~1LINE-B/status",
        json!("running"),
        &["no_station_running_unstaffed"],
    );
    assert_fails(
        "/roster/station~1CELL-R1/staffed",
        json!(false),
        &["no_station_running_unstaffed"],
    );
    assert_fails("/plant/line~1LINE-B/status", json!("down"), &[]);
}

#[test]
fn a_late_truck_needs_every_priority_order_noticed_and_recorded() {
    let order = ["priority_orders_noticed", "crm_records_updated"];
    assert_fails(
        "/hub/notice~12/order",
        json!("B-9"),
        &["priority_orders_noticed"],
    );
    assert_fails(
        "/approvals/change~1n2/decision/approved",
        json!(false),
        &order,
    );
    assert_fails("/crm/note~12/accounts", json!([]), &["crm_records_updated"]);
    assert_fails(
        "/crm/opportunity~12/closeDate",
        json!("2026-03-10"),
        &["crm_records_updated"],
    );
    assert_fails("/hub/inbound~1INB-0310-1/line_short_hours", json!(0.0), &[]);
}

#[test]
fn the_shift_report_names_every_front_that_opened() {
    let content = "/erp/Note~1abc/content";
    assert_fails(
        content,
        json!("RS-7731, Mei Tanaka, Priya Shah, INB-0310-1"),
        &["shift_report_in_erp"],
    );
    assert_fails(
        "/erp/Note~1abc/title",
        json!("Notes"),
        &["shift_report_in_erp"],
    );
    let mut short = handled();
    short["hub"]["inbound/INB-0310-1"]["line_short_hours"] = json!(0);
    short["erp"]["Note/abc"]["content"] = json!("LPL-DR-0310 RS-7731 Mei Tanaka Priya Shah");
    assert!(!failed(&short).contains(&"shift_report_in_erp".to_string()));
}

/// At the plant's scale (with the factory example's data next to the checkout; skipped without it).
#[test]
fn the_priority_orders_are_line_bs_due_within_a_day() {
    let Ok(data) = crate::facts::read_data(std::path::Path::new(crate::DEFAULT_DATA)) else {
        return;
    };
    let (data, _) = crate::scale::enlarge(&data, &crate::seed::company_extra());
    let p = Priority::of(&data);
    println!("priority orders: {:?} of {:?}", p.orders, p.customers);
    assert!(!p.orders.is_empty() && p.orders.len() <= 12, "{p:?}");
}
