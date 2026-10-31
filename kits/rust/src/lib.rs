//! Kit for writing arena integrations and benchmarks (protocol arena-part/1).
//!
//! Implement [`Part`] (or [`Benchmark`]) and call [`serve`] (or [`serve_benchmark`]).
mod error;
pub mod proto;
mod serve;

pub use error::PartError;
pub use serve::{serve, serve_benchmark, serve_benchmark_on, serve_on, Benchmark, Part};
