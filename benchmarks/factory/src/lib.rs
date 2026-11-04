//! The factory benchmark: a press bearing fails, and a team handles the incident across an ERP, the
//! outside companies, the plant and an approvals system. Scored by named checks over the end state.
pub mod checks;
pub mod facts;
pub mod seed;
pub mod team;
pub mod world;

use std::path::PathBuf;

use arena_part::proto::*;
use arena_part::{Benchmark, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

pub use facts::{Facts, DEFAULT_DATA};
pub use world::Backend;

type R<T> = Result<T, PartError>;

/// The one task.
pub const TASK: &str = "press-a1-bearing";

pub struct Factory {
    backend: Backend,
    data: PathBuf,
}

impl Factory {
    /// `data` is the factory's `ridgeline.json`; it is read when the world is seeded and scored.
    pub fn new(backend: Backend, data: PathBuf) -> Factory {
        Factory { backend, data }
    }

    fn data(&self) -> R<Value> {
        facts::read_data(&self.data).map_err(|e| PartError::invalid(format!("{e:#}")))
    }
}

fn task() -> TaskInfo {
    TaskInfo {
        id: TASK.into(),
        reason: "A press bearing fails: diagnose, source, approve, ship, schedule and notify"
            .into(),
        ticket: team::TICKET.into(),
    }
}

#[async_trait]
impl Benchmark for Factory {
    async fn describe(&self) -> R<BenchDescribe> {
        Ok(BenchDescribe {
            name: "factory".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            world: world::parts(self.backend),
            tasks: vec![task()],
            instructions: team::INSTRUCTIONS.into(),
            instructions_title: Some(team::INSTRUCTIONS_TITLE.into()),
            team: Some(team::team()),
            tools: vec![],
            links: world::links(),
            gates: world::gates(),
            scored: true,
        })
    }

    async fn seed(&self, p: SeedParams) -> R<()> {
        let erp = p
            .connections
            .get("erp")
            .ok_or_else(|| PartError::invalid("connections.erp is required"))?;
        let data = self.data()?;
        seed::seed(erp, &data)
            .await
            .map_err(|e| PartError::internal(format!("seeding the ERP: {e:#}")))
    }

    async fn call_tool(&self, p: BenchCallTool) -> R<BenchToolOutput> {
        Ok(BenchToolOutput {
            error: Some(format!("the factory benchmark has no tool {}", p.name)),
            part: "erp".into(),
            ..BenchToolOutput::default()
        })
    }

    /// None: the run's ids (a change's) are made up as it goes, so no list could name them. arenad
    /// doesn't ask a scored benchmark for them; `tests/` hold the path that passes.
    async fn gold_actions(&self, _p: GoldParams) -> R<GoldActions> {
        Ok(GoldActions::default())
    }

    async fn score(&self, p: ScoreParams) -> R<Score> {
        if p.task != TASK {
            return Err(PartError::invalid(format!("unknown task {}", p.task)));
        }
        let facts = Facts::of(&self.data()?).map_err(|e| PartError::invalid(format!("{e:#}")))?;
        let docs = json!(p.docs);
        Ok(Score {
            checks: checks::evaluate(&facts, &docs),
        })
    }
}
