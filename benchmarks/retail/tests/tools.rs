mod common;
use arena_part::{
    proto::{BenchCallTool, Effect, GoldParams},
    Benchmark,
};
use arena_retail::{
    load::{dump, with_conn},
    tools,
};
use common::*;
use serde_json::{json, Value};

/// tau2-bench retail's tools that change the database.
const WRITE_TOOLS: &[&str] = &[
    "cancel_pending_order",
    "exchange_delivered_order_items",
    "modify_pending_order_address",
    "modify_pending_order_items",
    "modify_pending_order_payment",
    "modify_user_address",
    "return_delivered_order_items",
];

fn changed_rows(base: &Value, now: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for t in ["users", "orders", "products"] {
        for (id, doc) in now[t].as_object().unwrap() {
            if base[t].get(id) != Some(doc) {
                out.insert(format!("{t}/{id}"), doc.clone());
            }
        }
    }
    out
}

fn eligible(task: &Value) -> bool {
    let ec = &task["evaluation_criteria"];
    let has_write = ec["actions"].as_array().is_some_and(|a| {
        a.iter()
            .any(|x| tools::is_write(x["name"].as_str().unwrap()))
    });
    let nl = ec
        .get("nl_assertions")
        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    has_write && !nl
}

async fn call(url: &str, name: &str, args: Value) -> arena_part::proto::BenchToolOutput {
    retail()
        .call_tool(BenchCallTool {
            name: name.into(),
            args,
            connections: conns(url),
        })
        .await
        .unwrap()
}

/// Truncates the database and reloads the `base` dump into it.
async fn reset_to(url: &str, base: &Value) {
    with_conn(url, async |c| {
        c.batch_execute("TRUNCATE users, orders, products").await?;
        for t in ["users", "orders", "products"] {
            c.execute(
                &format!("INSERT INTO {t} (id, doc) SELECT key, value FROM jsonb_each($1::jsonb)"),
                &[&base[t]],
            )
            .await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Runs the benchmark's gold actions for `task` against the database at `url`.
async fn run_gold(url: &str, task: &str) {
    let gold = retail()
        .gold_actions(GoldParams { task: task.into() })
        .await
        .unwrap();
    for a in gold.actions {
        call(url, &a.name, a.args).await;
    }
}

#[tokio::test]
async fn gold_changes_match_for_all_68_tasks() {
    let fix = fixture();
    assert_eq!(fix.len(), 68);
    let db = TestDb::seeded("gold").await;
    let base = with_conn(&db.url, async |c| dump(c).await).await.unwrap();
    let mut n = 0;
    for task in tasks().iter().filter(|t| eligible(t)) {
        let id = task["id"].as_str().unwrap();
        reset_to(&db.url, &base).await;
        // The gold actions come from the benchmark and run through its call_tool.
        run_gold(&db.url, id).await;
        let got = with_conn(&db.url, async |c| dump(c).await).await.unwrap();
        assert_eq!(
            Value::Object(changed_rows(&base, &got)),
            fix[id],
            "task {id}"
        );
        n += 1;
    }
    assert_eq!(n, 68);
}

#[tokio::test]
async fn gold_actions_for_27_nonempty() {
    let a = retail()
        .gold_actions(GoldParams { task: "27".into() })
        .await
        .unwrap();
    assert!(!a.actions.is_empty());
    assert!(a.actions.iter().all(|x| !x.name.is_empty()));
    let e = retail()
        .gold_actions(GoldParams {
            task: "nope".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32602);
}

#[tokio::test]
async fn reads_and_writes_are_recorded() {
    let db = TestDb::seeded("rw").await;
    let out = call(
        &db.url,
        "get_order_details",
        json!({"order_id": "#W2378156"}),
    )
    .await;
    assert_eq!(out.part, "retail");
    assert_eq!(out.reads, vec!["orders/#W2378156"]);
    assert!(out.writes.is_empty());
    assert_eq!(out.error, None);
    assert_eq!(out.result.unwrap()["order_id"], "#W2378156");

    let out = call(&db.url, "list_all_product_types", json!({})).await;
    assert_eq!(out.reads, vec!["products/*"]);
    let out = call(&db.url, "calculate", json!({"expression": "(1.5 + 2) * 3"})).await;
    assert_eq!(out.result, Some(json!("10.5")));
}

#[tokio::test]
async fn unknown_tool_and_bad_args_are_tool_errors() {
    let db = TestDb::seeded("err").await;
    let out = call(&db.url, "nope", json!({})).await;
    assert_eq!(out.error.as_deref(), Some("Unknown tool nope"));
    assert_eq!(out.part, "retail");
    assert!(call(&db.url, "get_order_details", json!({}))
        .await
        .error
        .is_some());
    assert!(call(&db.url, "get_order_details", json!({"order_id": 5}))
        .await
        .error
        .is_some());
    assert!(call(&db.url, "get_order_details", json!("x"))
        .await
        .error
        .is_some());
    let out = call(&db.url, "get_order_details", json!({"order_id": "#nope"})).await;
    assert_eq!(out.error.as_deref(), Some("Order not found"));
    assert_eq!(out.reads, vec!["orders/#nope"]);
}

#[tokio::test]
async fn describe_lists_tasks() {
    let d = retail().describe().await.unwrap();
    assert_eq!(d.name, "retail");
    assert_eq!(d.world.len(), 1);
    assert_eq!(
        (d.world[0].part.as_str(), d.world[0].integration.as_str()),
        ("retail", "postgres")
    );
    assert_eq!(d.tasks.len(), 10);
    assert!(d
        .tasks
        .iter()
        .all(|t| t.ticket.starts_with("# Customer ticket")));
    assert_eq!(
        d.instructions,
        std::fs::read_to_string(data_dir().join("policy.md")).unwrap()
    );
    assert_eq!(d.instructions_title.as_deref(), Some("Store policy"));
}

#[tokio::test]
async fn describe_suggests_triage_then_resolver() {
    let team = retail().describe().await.unwrap().team.expect("a team");
    let ids: Vec<&str> = team.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["triage", "resolver"]);
    assert!(team.agents[0]
        .prompt
        .starts_with("You are the triage agent"));
    assert!(team.agents[1].prompt.contains("handoff note"));
}

#[tokio::test]
async fn describe_lists_tools_and_their_effects() {
    let d = retail().describe().await.unwrap();
    for t in &d.tools {
        let want = if WRITE_TOOLS.contains(&t.name.as_str()) {
            Effect::Write
        } else {
            Effect::Read
        };
        assert_eq!(t.effect, want, "{}", t.name);
        assert_eq!(t.input_schema["type"], "object");
    }
    let mut names: Vec<&str> = d.tools.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    let mut want: Vec<&str> = tools::tool_names().collect();
    assert_eq!(want.len(), 16);
    want.sort();
    assert_eq!(names, want);
    assert!(!names.contains(&"handoff"));
}
