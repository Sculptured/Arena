//! Experiments in the metadata database: each experiment's definition, its setups being suites
//! (`suites.experiment_id`, in `suites.setup_ord` order).

use anyhow::Result;
use tokio_postgres::Row;

use super::{
    new_id,
    suites::{suite_from, SetupOf, SUITE_COLS},
    NewSuite, Store,
};
use crate::{
    experiment::Experiment,
    model::{Runner, Team},
    scenario::Scenario,
    suite::Suite,
};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS experiments (
    id text PRIMARY KEY,
    name text NOT NULL,
    task_id text NOT NULL,
    benchmark text NOT NULL,
    runs integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    ended_at timestamptz,
    error text
);
"#;

/// Experiments arenad was driving when it stopped get no more runs (their suites are built-in
/// ones, which `suites::INTERRUPT` ends).
pub(super) const INTERRUPT: &str = "UPDATE experiments SET ended_at = now(), \
     error = 'arenad stopped before the experiment finished' WHERE ended_at IS NULL";

const COLS: &str = "id, name, task_id, benchmark, runs, created_at, ended_at, error";

/// One setup of a new experiment: its name, team and runner.
pub struct NewSetup<'a> {
    pub name: &'a str,
    pub team: &'a Team,
    pub runner: Runner,
}

/// What a new experiment is: its task and scenario, how many runs each setup gets, and the
/// setups.
pub struct NewExperiment<'a> {
    pub name: &'a str,
    pub task: &'a str,
    pub runs: i32,
    pub benchmark: &'a str,
    pub scenario: &'a Scenario,
    pub setups: &'a [NewSetup<'a>],
}

fn experiment_from(r: &Row) -> Experiment {
    Experiment {
        id: r.get("id"),
        name: r.get("name"),
        task_id: r.get("task_id"),
        benchmark: r.get("benchmark"),
        runs: r.get("runs"),
        created_at: r.get("created_at"),
        ended_at: r.get("ended_at"),
        error: r.get("error"),
    }
}

impl Store {
    /// Stores a new experiment and one suite per setup (the task `runs` times in the scenario):
    /// the experiment and its suites in setup order.
    pub async fn create_experiment(
        &self,
        new: NewExperiment<'_>,
    ) -> Result<(Experiment, Vec<Suite>)> {
        let id = new_id();
        let sql = "INSERT INTO experiments (id, name, task_id, benchmark, runs)
                   VALUES ($1, $2, $3, $4, $5)";
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 5] =
            [&id, &new.name, &new.task, &new.benchmark, &new.runs];
        self.client.execute(sql, &params).await?;
        let tasks = [new.task.to_string()];
        let mut suites = vec![];
        for (ord, s) in (0..).zip(new.setups) {
            let suite = NewSuite {
                name: s.name,
                team: s.team,
                tasks: &tasks,
                repeats: new.runs,
                runner: s.runner,
                benchmark: new.benchmark,
                scenario: new.scenario,
                variants: &[],
            };
            let of = SetupOf {
                experiment: &id,
                ord,
            };
            suites.push(self.insert_suite(&suite, Some(of)).await?);
        }
        let stored = self.experiment(&id).await?;
        Ok((stored.expect("the experiment was just stored"), suites))
    }

    /// Every experiment, newest first.
    pub async fn experiments(&self) -> Result<Vec<Experiment>> {
        let sql = format!("SELECT {COLS} FROM experiments ORDER BY created_at DESC, id");
        let rows = self.client.query(&sql, &[]).await?;
        Ok(rows.iter().map(experiment_from).collect())
    }

    pub async fn experiment(&self, id: &str) -> Result<Option<Experiment>> {
        let sql = format!("SELECT {COLS} FROM experiments WHERE id = $1");
        let row = self.client.query_opt(&sql, &[&id]).await?;
        Ok(row.as_ref().map(experiment_from))
    }

    /// The experiment's suites, in setup order.
    pub async fn experiment_suites(&self, id: &str) -> Result<Vec<Suite>> {
        let sql =
            format!("SELECT {SUITE_COLS} FROM suites WHERE experiment_id = $1 ORDER BY setup_ord");
        let rows = self.client.query(&sql, &[&id]).await?;
        rows.iter().map(suite_from).collect()
    }

    /// True once the experiment ended (finished or stopped); a missing one counts as ended.
    pub async fn experiment_ended(&self, id: &str) -> Result<bool> {
        let sql = "SELECT ended_at IS NOT NULL AS ended FROM experiments WHERE id = $1";
        let row = self.client.query_opt(sql, &[&id]).await?;
        Ok(row.is_none_or(|r| r.get("ended")))
    }

    /// Records that the experiment ended, with why when it stopped early, and ends its suites
    /// that have not ended with the same why. Nothing changes for one that ended already.
    pub async fn end_experiment(&self, id: &str, error: Option<&str>) -> Result<()> {
        let sql = "WITH e AS (UPDATE experiments SET ended_at = now(), error = $2
                              WHERE id = $1 AND ended_at IS NULL RETURNING id)
                   UPDATE suites SET ended_at = now(), error = $2
                   FROM e WHERE suites.experiment_id = e.id AND suites.ended_at IS NULL";
        self.client.execute(sql, &[&id, &error]).await?;
        Ok(())
    }
}
