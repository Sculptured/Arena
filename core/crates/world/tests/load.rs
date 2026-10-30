use std::path::PathBuf;
use world::{dump, schema::load_base};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

#[tokio::test]
async fn base_round_trips() {
    let admin = world::testing::admin();
    load_base(&admin, &data_dir()).await.unwrap();
    load_base(&admin, &data_dir()).await.unwrap();

    let (client, conn) = tokio_postgres::connect(&admin.db_url("world_base"), tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(conn);
    let got = dump(&client).await.unwrap();

    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("db.json")).unwrap()).unwrap();
    assert_eq!(got, want);
    assert_eq!(got["users"].as_object().unwrap().len(), 500);
    assert_eq!(got["orders"].as_object().unwrap().len(), 1000);
    assert_eq!(got["products"].as_object().unwrap().len(), 50);
}
