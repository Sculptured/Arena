//! Suites in the metadata database: each suite's definition, and one row per task × repeat
//! naming the run that filled it.

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio_postgres::Row;

use super::{new_id, Store};
use crate::{
    model::{Runner, Team},
    scenario::Scenario,
    suite::{slots_of, variant_count, Slot, Suite},
};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS suites (
    id text PRIMARY KEY,
    name text NOT NULL,
    team jsonb NOT NULL,
    tasks jsonb NOT NULL,
    repeats integer NOT NULL,
    runner text NOT NULL,
    benchmark text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    ended_at timestamptz,
    error text
);
CREATE TABLE IF NOT EXISTS suite_runs (
    suite_id text NOT NULL REFERENCES suites(id),
    ord integer NOT NULL,
    task_id text NOT NULL,
    repeat integer NOT NULL,
    claimed boolean NOT NULL DEFAULT false,
    run_id text,
    branch_id text,
    error text,
    PRIMARY KEY (suite_id, ord)
);
ALTER TABLE suites ADD COLUMN IF NOT EXISTS scenario jsonb NOT NULL DEFAULT '{}';
ALTER TABLE suites ADD COLUMN IF NOT EXISTS variants jsonb NOT NULL DEFAULT '[]';
ALTER TABLE suite_runs ADD COLUMN IF NOT EXISTS variant integer NOT NULL DEFAULT 0;
ALTER TABLE suites ADD COLUMN IF NOT EXISTS experiment_id text;
ALTER TABLE suites ADD COLUMN IF NOT EXISTS setup_ord integer;
"#;

/// Built-in suites arenad was driving when it stopped get no more runs.
pub(super) const INTERRUPT: &str = "UPDATE suites SET ended_at = now(), \
     error = 'arenad stopped before the suite finished' \
     WHERE runner <> 'external' AND ended_at IS NULL";

pub(super) const SUITE_COLS: &str =
    "id, name, team, tasks, repeats, runner, benchmark, created_at, ended_at, error, scenario, variants, \
     experiment_id, setup_ord";
/// Inserts a suite (`$1`..`$7`, `$11` the scenario, `$12` the variants, `$14` its experiment and
/// `$15` its place there) and its slots (`$8`..`$10` and `$13`, in arrays).
const INSERT_SUITE: &str =
    "WITH s AS (INSERT INTO suites (id, name, team, tasks, repeats, runner, benchmark, scenario, variants,
                                   experiment_id, setup_ord)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $11, $12, $14, $15) RETURNING id)
           INSERT INTO suite_runs (suite_id, ord, task_id, repeat, variant)
           SELECT s.id, u.o, u.t, u.r, u.v
           FROM s, unnest($8::int[], $9::text[], $10::int[], $13::int[]) AS u(o, t, r, v)";
const SLOT_COLS: &str = "ord, task_id, variant, repeat, claimed, run_id, branch_id, error";

/// What a new suite is: its team, tasks, repeats, runner and benchmark.
pub struct NewSuite<'a> {
    pub name: &'a str,
    pub team: &'a Team,
    pub tasks: &'a [String],
    pub repeats: i32,
    pub runner: Runner,
    pub benchmark: &'a str,
    pub scenario: &'a Scenario,
    pub variants: &'a [Scenario],
}

/// Where an experiment's setup sits: the experiment, and the setup's place in it (from 0).
#[derive(Debug, Clone, Copy)]
pub(super) struct SetupOf<'a> {
    pub experiment: &'a str,
    pub ord: i32,
}

/// A new suite's slots as the arrays `INSERT_SUITE` unnests.
struct SlotColumns<'a> {
    ords: Vec<i32>,
    tasks: Vec<&'a str>,
    repeats: Vec<i32>,
    variants: Vec<i32>,
}

impl SlotColumns<'_> {
    fn of(slots: &[Slot]) -> SlotColumns<'_> {
        SlotColumns {
            ords: slots.iter().map(|s| s.ord).collect(),
            tasks: slots.iter().map(|s| s.task_id.as_str()).collect(),
            repeats: slots.iter().map(|s| s.repeat).collect(),
            variants: slots.iter().map(|s| s.variant).collect(),
        }
    }
}

fn runner_word(r: Runner) -> Result<String> {
    let v = serde_json::to_value(r)?;
    v.as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("a runner is a word"))
}

pub(super) fn suite_from(r: &Row) -> Result<Suite> {
    let team: Value = r.get("team");
    let tasks: Value = r.get("tasks");
    let runner: String = r.get("runner");
    let scenario: Value = r.get("scenario");
    let variants: Value = r.get("variants");
    Ok(Suite {
        id: r.get("id"),
        name: r.get("name"),
        team: serde_json::from_value(team)?,
        tasks: serde_json::from_value(tasks)?,
        repeats: r.get("repeats"),
        runner: serde_json::from_value(Value::String(runner))?,
        benchmark: r.get("benchmark"),
        created_at: r.get("created_at"),
        ended_at: r.get("ended_at"),
        error: r.get("error"),
        scenario: serde_json::from_value(scenario).unwrap_or_default(),
        variants: serde_json::from_value(variants).unwrap_or_default(),
        experiment_id: r.get("experiment_id"),
        setup_ord: r.get("setup_ord"),
    })
}

fn slot_from(r: &Row) -> Slot {
    Slot {
        ord: r.get("ord"),
        task_id: r.get("task_id"),
        variant: r.get("variant"),
        repeat: r.get("repeat"),
        claimed: r.get("claimed"),
        run_id: r.get("run_id"),
        branch_id: r.get("branch_id"),
        error: r.get("error"),
    }
}

impl Store {
    /// Stores a new suite and its slots, none claimed: the suite.
    pub async fn create_suite(&self, new: NewSuite<'_>) -> Result<Suite> {
        self.insert_suite(&new, None).await
    }

    /// Stores a new suite (a setup of an experiment, with `of`) and its slots: the suite.
    pub(super) async fn insert_suite(
        &self,
        new: &NewSuite<'_>,
        of: Option<SetupOf<'_>>,
    ) -> Result<Suite> {
        let id = new_id();
        let slots = slots_of(new.tasks, variant_count(new.variants), new.repeats);
        let cols = SlotColumns::of(&slots);
        let scenario = serde_json::to_value(new.scenario)?;
        let scenarios = serde_json::to_value(new.variants)?;
        let (experiment, ord) = (of.map(|o| o.experiment), of.map(|o| o.ord));
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 15] = [
            &id,
            &new.name,
            &serde_json::to_value(new.team)?,
            &serde_json::to_value(new.tasks)?,
            &new.repeats,
            &runner_word(new.runner)?,
            &new.benchmark,
            &cols.ords,
            &cols.tasks,
            &cols.repeats,
            &scenario,
            &scenarios,
            &cols.variants,
            &experiment,
            &ord,
        ];
        self.client.execute(INSERT_SUITE, &params).await?;
        self.suite(&id)
            .await?
            .ok_or_else(|| anyhow!("the suite just stored is gone"))
    }

    /// Every suite, newest first.
    pub async fn suites(&self) -> Result<Vec<Suite>> {
        let sql = format!("SELECT {SUITE_COLS} FROM suites ORDER BY created_at DESC, id");
        let rows = self.client.query(&sql, &[]).await?;
        rows.iter().map(suite_from).collect()
    }

    pub async fn suite(&self, id: &str) -> Result<Option<Suite>> {
        let sql = format!("SELECT {SUITE_COLS} FROM suites WHERE id = $1");
        let row = self.client.query_opt(&sql, &[&id]).await?;
        row.as_ref().map(suite_from).transpose()
    }

    /// The suite's slots, in run order.
    pub async fn slots(&self, suite: &str) -> Result<Vec<Slot>> {
        let sql = format!("SELECT {SLOT_COLS} FROM suite_runs WHERE suite_id = $1 ORDER BY ord");
        let rows = self.client.query(&sql, &[&suite]).await?;
        Ok(rows.iter().map(slot_from).collect())
    }

    /// Takes the suite's first unclaimed slot; None when every slot is claimed.
    pub async fn claim_slot(&self, suite: &str) -> Result<Option<Slot>> {
        let sql = format!(
            "UPDATE suite_runs SET claimed = true
             WHERE suite_id = $1 AND NOT claimed AND ord = (
                 SELECT min(ord) FROM suite_runs WHERE suite_id = $1 AND NOT claimed)
             RETURNING {SLOT_COLS}"
        );
        let row = self.client.query_opt(&sql, &[&suite]).await?;
        Ok(row.as_ref().map(slot_from))
    }

    /// Names the run (and its first branch) that fills a slot.
    pub async fn fill_slot(&self, suite: &str, ord: i32, run: &str, branch: &str) -> Result<()> {
        let sql =
            "UPDATE suite_runs SET run_id = $3, branch_id = $4 WHERE suite_id = $1 AND ord = $2";
        self.client
            .execute(sql, &[&suite, &ord, &run, &branch])
            .await?;
        Ok(())
    }

    /// Records why a slot's run could not start.
    pub async fn slot_failed(&self, suite: &str, ord: i32, error: &str) -> Result<()> {
        let sql = "UPDATE suite_runs SET error = $3 WHERE suite_id = $1 AND ord = $2";
        self.client.execute(sql, &[&suite, &ord, &error]).await?;
        Ok(())
    }

    /// Records that arenad stopped driving a built-in suite, with why when it stopped early.
    pub async fn end_suite(&self, suite: &str, error: Option<&str>) -> Result<()> {
        let sql = "UPDATE suites SET ended_at = now(), error = $2 WHERE id = $1";
        self.client.execute(sql, &[&suite, &error]).await?;
        Ok(())
    }
}
