//! MariaDB with saves as binlog ranges: a dedicated server with row-based binary logging, where
//! a save is a [`Manifest`] (an optional dump plus binlog segments) and a restore loads the dump
//! and replays the segments into a new database.
//!
//! - [`Server`]: start the server container if absent, query it, and run `mariadb-dump`,
//!   `mariadb-binlog` and `mariadb` inside it (`docker exec`, never through a shell).
//! - [`Manifest`], [`Position`]: the save chain and binlog arithmetic, with no server needed.
//! - [`Blobs`]: dumps stored once each by content hash.
mod blobs;
mod docker;
mod manifest;
mod position;
mod server;
mod sql;

pub use blobs::{Blobs, Freed};
pub use manifest::{Manifest, Segment, ServerRef, MANIFEST};
pub use mysql_async;
pub use position::{file_seq, purge_target, span, span_bytes, BinlogFile, Position, Purged};
pub use server::{valid_docker_name, Server, ServerConfig, IMAGE};
pub use sql::quote_ident;
