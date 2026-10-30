use world::testing::{admin, Cleanup};

// The guard must drop its databases even when the test body panics.
#[tokio::test]
async fn cleanup_drops_databases_on_panic() {
    let a = admin();
    let name = format!("world_tmp_cleanup_{}", std::process::id());
    a.drop_db(&name).await.unwrap();
    a.create_db_from(&name, "template1").await.unwrap();
    let n = name.clone();
    let result = std::panic::catch_unwind(move || {
        let g = Cleanup::new();
        g.add(&n);
        panic!("assertion failed in a test");
    });
    assert!(result.is_err());
    assert!(!a.exists(&name).await.unwrap(), "guard dropped the database during unwinding");
}
