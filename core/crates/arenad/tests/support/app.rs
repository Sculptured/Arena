//! The pieces every test app needs: a fresh metadata store (dropped with its databases by the
//! returned `Cleanup`), a listener on a free port, and serving the app's router on it.
#![allow(dead_code)]

use std::sync::Arc;

use arenad::{
    app::{self, AppState},
    store::Store,
};
use world::{snapshot::Worlds, testing::Cleanup};

/// A store in a fresh `arena_test_<kind>_<tag>_<id>` database.
pub async fn meta_store(worlds: &Worlds, kind: &str, tag: &str) -> (Arc<Store>, Cleanup) {
    let id = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let meta = format!("arena_test_{kind}_{tag}_{id}");
    let clean = Cleanup::new();
    clean.meta(&meta);
    (
        Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap()),
        clean,
    )
}

/// A listener on a free loopback port, and its base URL.
pub async fn bind() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    (listener, base)
}

/// Serves the app on `listener` for the rest of the test.
pub fn serve(listener: tokio::net::TcpListener, state: &Arc<AppState>) {
    let router = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
}
