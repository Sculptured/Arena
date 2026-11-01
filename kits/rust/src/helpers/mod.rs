//! Helpers for common kinds of integration. Most sit behind a cargo feature.
pub mod busy;
#[cfg(feature = "compose")]
pub mod compose;
pub mod docker_name;
#[cfg(feature = "folder")]
pub mod folder;
#[cfg(feature = "mariadb")]
pub mod mariadb;
#[cfg(feature = "postgres")]
pub mod pg;
mod pool;
pub mod process_group;
#[cfg(feature = "folder")]
pub mod save_dirs;
pub mod sql_tools;
#[cfg(feature = "compose")]
pub mod tar_digest;
#[cfg(feature = "folder")]
pub mod temp_dir;
mod tree;

pub use pool::blocking;
pub use tree::{remove_tree, remove_tree_or_log};
