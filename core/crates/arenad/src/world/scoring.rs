//! Scoring a world by the benchmark's named checks: every part's end state, read with `inspect`.

use parts::proto::{Docs, NamedCheck, Score};
use serde_json::{json, Map, Value};

use super::{call, World, WorldError};

impl World {
    /// `{part: {key: doc}}` of every part's live instance.
    async fn end_state(&self) -> Result<Map<String, Value>, WorldError> {
        let mut out = Map::new();
        for p in &self.parts {
            let params = json!({"target": {"instance": p.instance}, "keys": []});
            let docs: Docs = call(&p.client, &p.label(), "inspect", params).await?;
            out.insert(p.name.clone(), Value::Object(docs.docs));
        }
        Ok(out)
    }

    /// The benchmark's named checks of this world's end state for `task`.
    pub async fn score(&self, task: &str) -> Result<Vec<NamedCheck>, WorldError> {
        let params = json!({"task": task, "docs": self.end_state().await?});
        let score: Score = call(self.bench_client()?, &self.bench_label(), "score", params).await?;
        Ok(score.checks)
    }
}
