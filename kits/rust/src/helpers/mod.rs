//! Helpers for common kinds of integration. Most sit behind a cargo feature.
#[cfg(feature = "compose")]
pub mod compose;
#[cfg(feature = "folder")]
pub mod folder;
#[cfg(feature = "mariadb")]
pub mod mariadb;
#[cfg(feature = "postgres")]
pub mod pg;
#[cfg(feature = "folder")]
pub mod save_dirs;
pub mod sql_tools;
#[cfg(feature = "compose")]
pub mod tar_digest;
#[cfg(feature = "folder")]
pub mod temp_dir;
