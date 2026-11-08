//! The factory benchmark, two tasks at Ridgeline's Plant 2: a press bearing fails, and a team
//! handles the incident across an ERP, a CRM, the outside companies, the plant and an approvals
//! system; and a shift on which a scenario opens fronts at a simulated utility, OT network, roster
//! and the hub's trucks. Scored by named checks over the end state; the world's twin is its `view`.
pub mod checks;
pub mod company;
pub mod crm;
pub mod date;
pub mod erp;
pub mod facts;
pub mod options;
pub mod scale;
pub mod seed;
pub mod shift;
pub mod shift_checks;
pub mod single;
pub mod tasks;
pub mod team;
pub mod twin;
pub mod world;

use std::path::PathBuf;
use std::sync::OnceLock;

use arena_part::proto::*;
use arena_part::{Benchmark, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

pub use facts::{Facts, DEFAULT_DATA};
pub use tasks::{SHIFT_TASK, TASK};
pub use world::Backend;

type R<T> = Result<T, PartError>;

pub const VERSION: &str = "0.4.0";

pub struct Factory {
    backend: Backend,
    data: PathBuf,
    plant_config: Value,
    team: String,
    /// The data at the plant's scale, made once (the twin needs it after every writing step).
    scaled: OnceLock<Value>,
}

impl Factory {
    /// `data` is the factory's `ridgeline.json`; it is read when the world is seeded, scored and
    /// drawn.
    pub fn new(backend: Backend, data: PathBuf) -> Factory {
        Factory {
            backend,
            data,
            plant_config: json!({}),
            team: "single".into(),
            scaled: OnceLock::new(),
        }
    }

    /// The team offered as arenad's default (`team::named`: `single`, `incident` or `shift`).
    pub fn with_team(mut self, team: &str) -> Factory {
        self.team = team.into();
        self
    }

    /// The plant part's config in the world, e.g. `{"record": "run.mp4"}` for `isaac-sim`.
    pub fn with_plant_config(mut self, config: Value) -> Factory {
        self.plant_config = config;
        self
    }

    fn data(&self) -> R<Value> {
        facts::read_data(&self.data).map_err(|e| PartError::invalid(format!("{e:#}")))
    }

    /// The data at the plant's scale, with the generated customers and orders.
    fn scaled(&self) -> R<&Value> {
        if let Some(v) = self.scaled.get() {
            return Ok(v);
        }
        let (data, _) = scale::enlarge(&self.data()?, &seed::company_extra());
        Ok(self.scaled.get_or_init(|| data))
    }

    fn facts(&self) -> R<Facts> {
        Facts::of(self.scaled()?).map_err(|e| PartError::invalid(format!("{e:#}")))
    }
}

#[async_trait]
impl Benchmark for Factory {
    async fn describe(&self) -> R<BenchDescribe> {
        Ok(BenchDescribe {
            name: "factory".into(),
            version: VERSION.into(),
            protocol: PROTOCOL,
            world: world::parts(self.backend, self.plant_config.clone()),
            tasks: tasks::tasks(),
            instructions: team::INSTRUCTIONS.into(),
            instructions_title: Some(team::INSTRUCTIONS_TITLE.into()),
            team: team::named(&self.team),
            teams: vec![team::team(), team::shift_team()],
            tools: vec![],
            links: world::links(),
            gates: world::gates(),
            scored: true,
            scenario_options: Some(options::scenario_options()),
            view: Some(ViewInfo {
                docs: twin::DOC_PARTS.map(String::from).to_vec(),
            }),
        })
    }

    async fn seed(&self, p: SeedParams) -> R<()> {
        let erp = p
            .connections
            .get("erp")
            .ok_or_else(|| PartError::invalid("connections.erp is required"))?;
        let data = self.data()?;
        let crm = p
            .connections
            .get("crm")
            .ok_or_else(|| PartError::invalid("connections.crm is required"))?;
        seed::seed(erp, crm, &data)
            .await
            .map_err(|e| PartError::internal(format!("seeding the company: {e:#}")))
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
        let docs = json!(p.docs);
        let checks = match p.task.as_str() {
            TASK => checks::evaluate(&self.facts()?, &docs),
            SHIFT_TASK => {
                shift_checks::evaluate(&shift_checks::Priority::of(self.scaled()?), &docs)
            }
            other => return Err(PartError::invalid(format!("unknown task {other}"))),
        };
        Ok(Score { checks })
    }

    /// The digital twin, from the parts' docs alone.
    async fn view(&self, p: ViewParams) -> R<ViewResult> {
        let priority = shift_checks::Priority::of(self.scaled()?);
        let view = twin::draw(&json!(p.docs), &self.facts()?, &priority);
        Ok(ViewResult { view })
    }
}
