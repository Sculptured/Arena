//! Helpers for common kinds of integration. Each sits behind a cargo feature.
#[cfg(feature = "compose")]
pub mod compose;
#[cfg(feature = "folder")]
pub mod folder;
#[cfg(feature = "postgres")]
pub mod pg;
#[cfg(feature = "folder")]
pub mod save_dirs;
#[cfg(feature = "folder")]
pub mod temp_dir;
