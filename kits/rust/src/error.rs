//! Errors a part reports to arenad as JSON-RPC errors.
use std::fmt;

/// A JSON-RPC error with the arena-part/1 codes.
#[derive(Debug, Clone, PartialEq)]
pub struct PartError {
    pub code: i64,
    pub message: String,
}

impl PartError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    /// -32001: can't do that restore.
    pub fn cant_restore(message: impl Into<String>) -> Self {
        Self::new(-32001, message)
    }
    /// -32002: unknown instance or save.
    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(-32002, message)
    }
    /// -32002 for an instance id the part does not run.
    pub fn unknown_instance(id: &str) -> Self {
        Self::unknown(format!("unknown instance {id}"))
    }
    /// -32003: fingerprint mismatch.
    pub fn fingerprint(message: impl Into<String>) -> Self {
        Self::new(-32003, message)
    }
    /// -32602: invalid params.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
    /// -32603: internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }
    /// -32603 from any error, with its causes (`{e:#}`).
    pub fn internal_from(e: impl fmt::Display) -> Self {
        Self::internal(format!("{e:#}"))
    }
    /// -32601: method not found.
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for PartError {}

impl From<tokio::task::JoinError> for PartError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::internal_from(e)
    }
}

impl From<anyhow::Error> for PartError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(format!("{e:#}"))
    }
}
