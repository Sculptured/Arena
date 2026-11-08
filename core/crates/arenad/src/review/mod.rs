//! AI review: a real model reads a compact digest of a run (or of two runs, or two suites, compared)
//! and says where it went wrong and what to change (or what matters between the two). Optional:
//! off means nothing is sent to a model ([`settings`]). Reviews are stored and not recomputed
//! unless asked.
//!
//! - [`digest`], [`pair`], [`suites`]: what the reviewer reads, built from the record, bounded.
//! - [`prompt`]: what it is told, and the schema of its answer.
//! - [`reviewer`]: the call to the model (Claude Code headless, or one of arenad's providers).
//! - [`output`]: the answer checked: fields validated, citations checked against the record.

pub mod digest;
pub mod output;
pub mod pair;
pub mod prompt;
pub mod reviewer;
pub mod settings;
pub mod suites;
mod text;

use std::sync::Arc;

use anyhow::Result;

use crate::{app::AppState, store::ReviewDone};
use output::{Citable, Cites};
use prompt::Prompt;
use settings::ReviewerModel;

pub fn branch_key(id: &str) -> String {
    format!("branch:{id}")
}

pub fn compare_key(a: &str, b: &str) -> String {
    format!("compare:{a}:{b}")
}

pub fn suites_key(a: &str, b: &str) -> String {
    format!("suites:{a}:{b}")
}

/// How an answer is checked: as a run review, or as a comparison citing steps or runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Run,
    Compare(Cites),
}

impl Shape {
    /// The stored review's kind.
    pub fn kind(self) -> &'static str {
        match self {
            Shape::Run => "run",
            Shape::Compare(Cites::Steps) => "compare",
            Shape::Compare(Cites::Runs) => "suites",
        }
    }
}

/// One review to make: the prompt, what it may cite, and how its answer is checked.
pub struct Job {
    pub prompt: Prompt,
    pub citable: Citable,
    pub shape: Shape,
}

/// The answer checked against what the job may cite: the review as JSON, and the issues found.
pub fn check(answer: &serde_json::Value, job: &Job) -> Result<(serde_json::Value, Vec<String>)> {
    let (v, issues) = match job.shape {
        Shape::Run => {
            let (r, i) = output::run_review(answer, &job.citable)?;
            (serde_json::to_value(r)?, i)
        }
        Shape::Compare(cites) => {
            let (r, i) = output::compare_review(answer, &job.citable, cites)?;
            (serde_json::to_value(r)?, i)
        }
    };
    Ok((v, issues.0))
}

/// Asks the reviewer and stores its checked answer under `key`, or why it failed.
pub async fn run(state: Arc<AppState>, key: String, model: ReviewerModel, job: Result<Job>) {
    let outcome = answer(&state, &key, &model, job).await;
    if let Err(e) = outcome {
        if let Err(e2) = state.store.fail_review(&key, &format!("{e:#}")).await {
            eprintln!("review {key}: {e:#}; storing that failed too: {e2:#}");
        }
    }
}

async fn answer(
    state: &AppState,
    key: &str,
    model: &ReviewerModel,
    job: Result<Job>,
) -> Result<()> {
    let job = job?;
    let cfg = &state.config.providers;
    let a = reviewer::ask(model, &job.prompt, cfg, &state.http).await?;
    let (review, issues) = check(&a.value, &job)?;
    let done = ReviewDone {
        review: &review,
        issues: &issues,
        cost_usd: a.cost_usd,
        cost_source: a.cost_source.as_deref(),
        tokens: a.tokens,
        seconds: a.seconds,
        digest_bytes: job.prompt.user.len(),
    };
    state.store.finish_review(key, done).await
}
