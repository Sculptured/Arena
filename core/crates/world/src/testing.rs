use crate::pg::Admin;

/// Test helper: connects using `ARENA_PG_URL` (default: the compose Postgres on 5544).
pub fn admin() -> Admin {
    let url = std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".to_string());
    Admin::lazy(&url)
}
