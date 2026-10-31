//! The pieces every test app needs: a fresh metadata store (dropped by the returned `Cleanup`), a
//! listener on a free port, and serving the app's router on it.
#![allow(dead_code)]

use std::sync::Arc;

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{self, AppState, Config},
    model::AgentKind,
    store::Store,
};
use parts::Hub;

use crate::pg::{admin, Cleanup};

/// A store in a fresh `arena_test_<kind>_<tag>_<id>` database.
pub async fn meta_store(kind: &str, tag: &str) -> (Arc<Store>, Cleanup) {
    let id = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let meta = format!("arena_test_{kind}_{tag}_{id}");
    let clean = Cleanup::new();
    clean.add(&meta);
    let store = Store::open_named(&admin(), &meta).await.unwrap();
    (Arc::new(store), clean)
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

/// An app on `hub` whose open agents talk to the (fake) upstream at `upstream`, served on a free
/// port; `tweak` adjusts the config first. The state and the app's base URL.
pub async fn open_app(
    store: Arc<Store>,
    hub: Arc<Hub>,
    upstream: &str,
    tweak: impl FnOnce(&mut Config),
) -> (Arc<AppState>, String) {
    let (listener, base) = bind().await;
    let mut config = Config::new();
    config.upstream_base = upstream.to_string();
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = base.clone();
    tweak(&mut config);
    let state = Arc::new(AppState::new(store, hub, config));
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    serve(listener, &state);
    (state, base)
}

/// The status and JSON body of a request to the app.
pub async fn send(req: reqwest::RequestBuilder) -> (u16, serde_json::Value) {
    let r = req.send().await.unwrap();
    (r.status().as_u16(), r.json().await.unwrap())
}
