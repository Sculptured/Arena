//! The pharma benchmark: a Grade A environmental-monitoring excursion during the fill of a
//! sterile biologic's batch, handled across an ERP (ERPNext), a CRM (Twenty), a simulated LIMS,
//! simulated cold-chain carriers and an approvals system (Sculptured), in five scenarios. Scored by
//! named outcome checks and hard safety checks over the end state.
pub mod checks;
pub mod crm;
pub mod data;
pub mod days;
pub mod erp;
pub mod erp_docs;
pub mod facts;
pub mod reports;
pub mod scenario;
pub mod seed;
pub mod team;
pub mod world;

use arena_part::proto::*;
use arena_part::{Benchmark, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

type R<T> = Result<T, PartError>;

pub const VERSION: &str = "0.1.0";

/// The benchmark; `team` is the deployment of `data/teams.json` it offers as arenad's default
/// team (`team` or `single`).
pub struct Pharma {
    pub team: String,
}

impl Default for Pharma {
    fn default() -> Pharma {
        Pharma {
            team: "team".into(),
        }
    }
}

fn connection<'a>(c: &'a serde_json::Map<String, Value>, part: &str) -> R<&'a Value> {
    c.get(part)
        .ok_or_else(|| PartError::invalid(format!("connections.{part} is required")))
}

#[async_trait]
impl Benchmark for Pharma {
    async fn describe(&self) -> R<BenchDescribe> {
        let d = data::data();
        Ok(BenchDescribe {
            name: "pharma".into(),
            version: VERSION.into(),
            protocol: PROTOCOL,
            world: world::parts(d),
            tasks: scenario::tasks(d),
            instructions: team::INSTRUCTIONS.into(),
            instructions_title: Some(team::INSTRUCTIONS_TITLE.into()),
            team: Some(team::bench_team(&self.team)),
            tools: reports::schemas(),
            links: world::links(),
            gates: world::gates(),
            scored: true,
        })
    }

    async fn seed(&self, p: SeedParams) -> R<()> {
        let (erp, crm) = (
            connection(&p.connections, "erp")?,
            connection(&p.connections, "crm")?,
        );
        seed::seed(erp, crm, data::data())
            .await
            .map_err(|e| PartError::internal(format!("seeding Larchmont Biologics: {e:#}")))
    }

    /// The reports, read from the ERP (so every call, refused or not, is the ERP's).
    async fn call_tool(&self, p: BenchCallTool) -> R<BenchToolOutput> {
        let mut out = BenchToolOutput {
            part: "erp".into(),
            ..BenchToolOutput::default()
        };
        if !reports::has(&p.name) {
            out.error = Some(format!("the pharma benchmark has no tool {}", p.name));
            return Ok(out);
        }
        let erp = erp::Erp::connect(connection(&p.connections, "erp")?)
            .map_err(|e| PartError::invalid(format!("{e:#}")))?;
        let args = if p.args.is_null() { json!({}) } else { p.args };
        match reports::run(&erp, data::data(), &p.name, &args).await {
            Ok((result, reads)) => (out.result, out.reads) = (Some(result), reads),
            Err(e) => out.error = Some(e),
        }
        Ok(out)
    }

    /// None: a run's ids are made up as it goes; `tests/` and the live test hold the paths.
    async fn gold_actions(&self, _p: GoldParams) -> R<GoldActions> {
        Ok(GoldActions::default())
    }

    async fn score(&self, p: ScoreParams) -> R<Score> {
        let s = scenario::of_task(&p.task)
            .ok_or_else(|| PartError::invalid(format!("unknown task {}", p.task)))?;
        let f = facts::Facts::new(data::data(), s.name);
        Ok(Score {
            checks: checks::evaluate(&f, &json!(p.docs)),
        })
    }
}
