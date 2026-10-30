use std::path::PathBuf;

use serde_json::{json, Value};
use world::{dump, schema::load_base, tools};

// Serializes the world_base copy step (Postgres refuses to copy a template in use).
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

fn tasks() -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(data_dir().join("tasks.json")).unwrap()).unwrap()
}

fn fixture() -> serde_json::Map<String, Value> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gold_changes.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

async fn temp_copy(admin: &world::pg::Admin, tag: &str) -> String {
    let _g = LOCK.lock().await;
    load_base(admin, &data_dir()).await.unwrap();
    let name = format!("world_tools_{}_{}", std::process::id(), tag);
    admin.drop_db(&name).await.unwrap();
    admin.create_db_from(&name, "world_base").await.unwrap();
    name
}

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

fn gold_actions(task: &Value) -> Vec<(String, Value)> {
    task["evaluation_criteria"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["name"].as_str().unwrap().to_string(), a["arguments"].clone()))
        .collect()
}

fn eligible(task: &Value) -> bool {
    let ec = &task["evaluation_criteria"];
    let has_write = ec["actions"].as_array().is_some_and(|a| {
        a.iter().any(|x| tools::WRITE_TOOLS.contains(&x["name"].as_str().unwrap()))
    });
    let nl = ec.get("nl_assertions").is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    has_write && !nl
}

#[tokio::test]
async fn gold_changes_match_for_all_68_tasks() {
    let admin = world::testing::admin();
    let fix = fixture();
    assert_eq!(fix.len(), 68);
    let db = temp_copy(&admin, "gold").await;
    // Dump base from the private copy, never from world_base itself.
    let base = admin.with_db(&db, async |c| dump(c).await).await.unwrap();
    let mut n = 0;
    for task in tasks().iter().filter(|t| eligible(t)) {
        let id = task["id"].as_str().unwrap();
        // Reset to base between tasks by running each in a rolled-back state: apply, dump, undo.
        let got = admin
            .with_db(&db, async |c| {
                c.batch_execute("TRUNCATE users, orders, products").await?;
                for t in ["users", "orders", "products"] {
                    c.execute(
                        &format!("INSERT INTO {t} (id, doc) SELECT key, value FROM jsonb_each($1::jsonb)"),
                        &[&base[t]],
                    )
                    .await?;
                }
                for (name, args) in gold_actions(task) {
                    tools::call(c, &name, &args).await?;
                }
                dump(c).await
            })
            .await
            .unwrap();
        assert_eq!(Value::Object(changed_rows(&base, &got)), fix[id], "task {id}");
        n += 1;
    }
    assert_eq!(n, 68);
    admin.drop_db(&db).await.unwrap();
}

#[tokio::test]
async fn ensure_gold_creates_and_caches() {
    let admin = world::testing::admin();
    let copy = temp_copy(&admin, "unused").await;
    let base = admin.with_db(&copy, async |c| dump(c).await).await.unwrap();
    admin.drop_db(&copy).await.unwrap();
    let fix = fixture();
    let task = tasks().into_iter().find(|t| eligible(t)).unwrap();
    let id = task["id"].as_str().unwrap().to_string();
    let name = format!("gold_{id}");
    admin.drop_db(&name).await.unwrap();
    let got = {
        let _g = LOCK.lock().await;
        world::gold::ensure_gold(&admin, &task).await.unwrap()
    };
    assert_eq!(got, name);
    let again = world::gold::ensure_gold(&admin, &task).await.unwrap();
    assert_eq!(again, name);
    let now = admin.with_db(&name, async |c| dump(c).await).await.unwrap();
    assert_eq!(Value::Object(changed_rows(&base, &now)), fix[&id]);
    admin.drop_db(&name).await.unwrap();
}

#[tokio::test]
async fn reads_and_writes_are_recorded() {
    let admin = world::testing::admin();
    let db = temp_copy(&admin, "rw").await;
    admin
        .with_db(&db, async |c| {
            let out = tools::call(c, "get_order_details", &json!({"order_id": "#W2378156"})).await?;
            assert_eq!(out.reads, vec!["orders/#W2378156"]);
            assert!(out.writes.is_empty());
            assert_eq!(out.error, None);
            assert_eq!(out.result.unwrap()["order_id"], "#W2378156");

            let out = tools::call(c, "list_all_product_types", &json!({})).await?;
            assert_eq!(out.reads, vec!["products/*"]);
            let out = tools::call(c, "calculate", &json!({"expression": "(1.5 + 2) * 3"})).await?;
            assert_eq!(out.result, Some(json!("10.5")));
            Ok(())
        })
        .await
        .unwrap();
    admin.drop_db(&db).await.unwrap();
}

#[tokio::test]
async fn unknown_tool_and_bad_args_are_tool_errors() {
    let admin = world::testing::admin();
    let db = temp_copy(&admin, "err").await;
    admin
        .with_db(&db, async |c| {
            let out = tools::call(c, "nope", &json!({})).await?;
            assert_eq!(out.error.as_deref(), Some("Unknown tool nope"));
            let out = tools::call(c, "get_order_details", &json!({})).await?;
            assert!(out.error.is_some());
            let out = tools::call(c, "get_order_details", &json!({"order_id": 5})).await?;
            assert!(out.error.is_some());
            let out = tools::call(c, "get_order_details", &json!("x")).await?;
            assert!(out.error.is_some());
            let out = tools::call(c, "get_order_details", &json!({"order_id": "#nope"})).await?;
            assert_eq!(out.error.as_deref(), Some("Order not found"));
            assert_eq!(out.reads, vec!["orders/#nope"]);
            Ok(())
        })
        .await
        .unwrap();
    admin.drop_db(&db).await.unwrap();
}

#[test]
fn schemas_cover_every_tool() {
    let s = tools::tool_schemas();
    let mut names: Vec<&str> = s.iter().map(|v| v["name"].as_str().unwrap()).collect();
    names.sort();
    let mut want: Vec<&str> = tools::READ_TOOLS.iter().chain(tools::WRITE_TOOLS).copied().collect();
    want.sort();
    assert_eq!(names, want);
    for v in &s {
        assert_eq!(v["inputSchema"]["type"], "object");
    }
}
