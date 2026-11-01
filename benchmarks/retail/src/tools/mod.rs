//! Retail tools on Postgres: a port of `cs2680-gap-tests/harness/tool.py` (`Retail` and `call`),
//! which was verified to reproduce tau2-bench's gold databases. Each call runs in one
//! transaction; rows are `SELECT doc ... FOR UPDATE`d (write tools), mutated as JSON, and
//! written back only when the tool succeeds.
//!
//! - `schema`: the tool list, their arguments, and the check of a call's arguments
//! - `retail`: the transaction view over the rows, and the read tools
//! - `orders`: the tools that change orders and users
//! - `call`: one tool call, in one transaction
//! - `calc`, `fail`, `value`: the `calculate` evaluator, the error type, and small helpers

mod calc;
mod call;
mod fail;
mod orders;
mod retail;
mod schema;
mod value;

pub use call::{call, ToolOutcome};
pub use schema::{is_write, tool_names, tool_schemas};
pub use value::fold;
