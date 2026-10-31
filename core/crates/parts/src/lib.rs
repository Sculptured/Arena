//! Installed parts (integrations and benchmarks): manifests, the program client and the hub.
mod check;
mod client;
mod hub;
mod manifest;

pub use check::{check, Report, Step};
pub use client::{CallError, Client};
pub use hub::Hub;
pub use manifest::{Kind, Manifest, Needs, Registry, Run};
