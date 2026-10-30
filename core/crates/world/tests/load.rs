use std::path::PathBuf;
use world::{dump, schema::load_base};

// Tests share world_base; serialize them so a copy never races another test's connection.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

#[tokio::test]
async fn base_round_trips() {
    let _g = LOCK.lock().await;
    let admin = world::testing::admin();
    load_base(&admin, &data_dir()).await.unwrap();
    load_base(&admin, &data_dir()).await.unwrap();

    let got = admin
        .with_db("world_base", async |c| dump(c).await)
        .await
        .unwrap();

    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("db.json")).unwrap()).unwrap();
    assert_eq!(got, want);
    assert_eq!(got["users"].as_object().unwrap().len(), 500);
    assert_eq!(got["orders"].as_object().unwrap().len(), 1000);
    assert_eq!(got["products"].as_object().unwrap().len(), 50);
}

#[tokio::test]
async fn copy_right_after_load_succeeds() {
    let _g = LOCK.lock().await;
    let admin = world::testing::admin();
    load_base(&admin, &data_dir()).await.unwrap();
    let name = format!("world_tmp_{}", std::process::id());
    let _guard = world::testing::Cleanup::new();
    _guard.add(&name);
    admin.drop_db(&name).await.unwrap();
    admin.create_db_from(&name, "world_base").await.unwrap();
    assert!(admin.exists(&name).await.unwrap());
    admin.drop_db(&name).await.unwrap();
    assert!(!admin.exists(&name).await.unwrap());
}
