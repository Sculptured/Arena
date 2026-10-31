//! Helpers for common kinds of integration. Each sits behind a cargo feature.
#[cfg(feature = "postgres")]
pub mod pg;
