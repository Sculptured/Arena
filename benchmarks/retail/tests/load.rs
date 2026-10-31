mod common;
use arena_retail::load::{dump, with_conn};
use common::*;

#[tokio::test]
async fn base_round_trips() {
    let db = TestDb::seeded("load").await;
    // Seeding again is a no-op.
    use arena_part::{proto::SeedParams, Benchmark};
    retail()
        .seed(SeedParams {
            connections: conns(&db.url),
        })
        .await
        .unwrap();

    let got = with_conn(&db.url, async |c| dump(c).await).await.unwrap();
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("db.json")).unwrap())
            .unwrap();
    assert_eq!(got, want);
    assert_eq!(got["users"].as_object().unwrap().len(), 500);
    assert_eq!(got["orders"].as_object().unwrap().len(), 1000);
    assert_eq!(got["products"].as_object().unwrap().len(), 50);
}

#[tokio::test]
async fn seed_needs_a_retail_connection() {
    use arena_part::{proto::SeedParams, Benchmark};
    let e = retail()
        .seed(SeedParams {
            connections: Default::default(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32602);
}
