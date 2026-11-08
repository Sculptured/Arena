//! Agents that stay alive across turns, for the dispatch runner: each turn starts with a new user
//! message and ends when the agent stops; the agent keeps its context between turns. Claude Code
//! keeps one long-lived headless process (`claude_session`); arenad's own loop keeps the
//! conversation (`open::OpenSession`).

use std::{future::Future, pin::Pin, sync::Arc};

use super::{AgentResult, FailureKind};
use crate::{exec::BranchCtx, model::AgentSpec, store::Store};

/// How one turn ended: well (with what the agent said last), or failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub ok: bool,
    pub error: Option<String>,
    /// `Some(Model)` when the model's own behaviour failed the turn; `None` on failure is an
    /// infrastructure error.
    pub failure: Option<FailureKind>,
    /// What the agent said at the end of the turn.
    pub text: String,
}

impl Turn {
    pub fn done(text: String) -> Turn {
        Turn {
            ok: true,
            error: None,
            failure: None,
            text,
        }
    }

    pub fn failed(error: String, failure: Option<FailureKind>) -> Turn {
        Turn {
            ok: false,
            error: Some(error),
            failure,
            text: String::new(),
        }
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One agent alive across turns.
pub trait Session: Send {
    /// Runs one turn with `input` as the user's message; resolves when the agent ends the turn.
    /// Dropping the future cuts the turn off; the session can still be ended.
    fn turn<'a>(&'a mut self, input: String) -> BoxFuture<'a, Turn>;

    /// Ends the session: the agent's tokens and cost over every turn, and the time it spent in
    /// them. `cut` says a turn was cut off when the run ended.
    fn end(self: Box<Self>, cut: bool) -> BoxFuture<'static, AgentResult>;
}

/// What a session starts with: its branch (for its tools and flow), its agent, and the store its
/// model calls are recorded in.
pub struct SessionStart {
    pub branch: Arc<BranchCtx>,
    pub spec: AgentSpec,
    pub store: Arc<Store>,
}

/// The note an agent's end carries when a turn of it was cut off.
pub const CUT_OFF: &str =
    "stopped in the middle of a turn when the run ended; that turn's cost is not counted";

/// The error of a session's end: the last failure, or that a turn was cut off.
pub fn end_error(last: Option<String>, cut: bool) -> Option<String> {
    last.or_else(|| cut.then(|| CUT_OFF.to_string()))
}
