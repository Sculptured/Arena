//! The retail benchmark: τ²-bench retail tools over a Postgres world part.
pub mod load;
pub mod tasks;
pub mod team;
pub mod tools;

use std::path::{Path, PathBuf};

use arena_part::proto::*;
use arena_part::{Benchmark, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

type R<T> = Result<T, PartError>;

/// `ARENA_RETAIL_DATA` if set (tests and dev), else `../../data/retail` from the working
/// directory (the hub runs the program in its manifest folder).
pub fn data_dir_from_env() -> PathBuf {
    std::env::var_os("ARENA_RETAIL_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../data/retail"))
}

pub struct Retail {
    data_dir: PathBuf,
    tasks: tasks::Tasks,
    policy: String,
}

impl Retail {
    pub fn new(data_dir: &Path) -> anyhow::Result<Retail> {
        let tasks = tasks::Tasks::load(data_dir)?;
        let p = data_dir.join("policy.md");
        let policy = std::fs::read_to_string(&p)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", p.display()))?;
        Ok(Retail {
            data_dir: data_dir.to_path_buf(),
            tasks,
            policy,
        })
    }
}

fn url_of(connections: &serde_json::Map<String, Value>) -> R<&str> {
    connections
        .get("retail")
        .and_then(|c| c["url"].as_str())
        .ok_or_else(|| PartError::invalid("connections.retail.url is required"))
}

fn schemas() -> Vec<ToolSchema> {
    tools::tool_schemas()
        .into_iter()
        .map(|v| {
            let name = v["name"].as_str().unwrap_or("").to_string();
            ToolSchema {
                effect: if tools::is_write(&name) {
                    Effect::Write
                } else {
                    Effect::Read
                },
                description: v["description"].as_str().unwrap_or("").to_string(),
                input_schema: v["inputSchema"].clone(),
                name,
            }
        })
        .collect()
}

#[async_trait]
impl Benchmark for Retail {
    async fn describe(&self) -> R<BenchDescribe> {
        let tasks = self
            .tasks
            .demo()
            .into_iter()
            .map(|(id, reason)| {
                let ticket = self.tasks.ticket(&id).unwrap_or_default();
                TaskInfo { id, reason, ticket }
            })
            .collect();
        Ok(BenchDescribe {
            name: "retail".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            world: vec![WorldPart {
                part: "retail".into(),
                integration: "postgres".into(),
                config: json!({}),
            }],
            tasks,
            instructions: self.policy.clone(),
            instructions_title: Some(team::INSTRUCTIONS_TITLE.into()),
            team: Some(team::team()),
            tools: schemas(),
            links: vec![],
        })
    }

    async fn seed(&self, p: SeedParams) -> R<()> {
        let url = url_of(&p.connections)?;
        Ok(load::seed(url, &self.data_dir).await?)
    }

    async fn call_tool(&self, p: BenchCallTool) -> R<BenchToolOutput> {
        let url = url_of(&p.connections)?;
        let args = if p.args.is_null() { json!({}) } else { p.args };
        let out = load::with_conn(url, async |c| tools::call(c, &p.name, &args).await).await?;
        Ok(BenchToolOutput {
            result: out.result,
            error: out.error,
            part: "retail".into(),
            reads: out.reads,
            writes: out.writes,
        })
    }

    async fn gold_actions(&self, p: GoldParams) -> R<GoldActions> {
        let actions = self
            .tasks
            .gold_actions(&p.task)
            .ok_or_else(|| PartError::invalid(format!("unknown task {}", p.task)))?;
        Ok(GoldActions {
            actions: actions
                .into_iter()
                .map(|(name, args)| Action { name, args })
                .collect(),
        })
    }
}
