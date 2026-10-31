//! Saving a world's parts: after a step (the part written to and the clock parts that changed),
//! all of them for a final state or under fixed names, and comparing them with saves. Parts that
//! keep no saves (only replay) are never saved; parts with replay are not in the shared saves,
//! so they are not compared with them.

use parts::proto::Differing;
use serde_json::{json, Value};

use super::{call, new_save_id, Part, Saves, World, WorldError};

/// What saving after a step gave: the new saves, and the first failure (the other parts are
/// still saved).
#[derive(Debug, Default)]
pub struct StepSaves {
    pub saves: Saves,
    pub error: Option<WorldError>,
}

impl World {
    async fn save_as(&self, p: &Part, save: &str) -> Result<(), WorldError> {
        let dir = self.hub.save_dir(&p.integration, save);
        let _: Value = call(
            &p.client,
            &p.label(),
            "save",
            json!({"instance": p.instance, "save": save, "dir": dir}),
        )
        .await?;
        Ok(())
    }

    /// Saves `p` under a new id and records it as the part's current state.
    async fn save_new(&self, p: &Part) -> Result<String, WorldError> {
        let save = new_save_id();
        let saved = self.save_as(p, &save).await;
        let mut t = self.track.lock().unwrap();
        match saved {
            Ok(()) => {
                t.last.insert(p.name.clone(), save.clone());
                t.dirty.remove(&p.name);
                t.changed.remove(&p.name);
                Ok(save)
            }
            Err(e) => {
                t.dirty.insert(p.name.clone());
                Err(e)
            }
        }
    }

    /// The parts to save after a step: the part written to, and every clock part that changed
    /// since its last save; only parts that keep saves.
    fn changed_parts(&self, written_part: Option<&str>) -> Vec<&Part> {
        let t = self.track.lock().unwrap();
        self.parts
            .iter()
            .filter(|p| written_part == Some(p.name.as_str()) || t.changed.contains(&p.name))
            .filter(|p| p.keeps_saves())
            .collect()
    }

    /// Saves the parts that changed with a step (see `changed_parts`). A part whose save fails
    /// stays unsaved until its next one.
    pub async fn save_changed(&self, written_part: Option<&str>) -> StepSaves {
        let mut out = StepSaves::default();
        for p in self.changed_parts(written_part) {
            match self.save_new(p).await {
                Ok(save) => {
                    out.saves.insert(p.name.clone(), save);
                }
                Err(e) => {
                    out.error.get_or_insert(e);
                }
            }
        }
        out
    }

    /// The part's last save, when nothing changed since.
    fn reusable_save(&self, p: &Part) -> Option<String> {
        let t = self.track.lock().unwrap();
        let unchanged = !t.dirty.contains(&p.name) && !t.changed.contains(&p.name);
        t.last.get(&p.name).filter(|_| unchanged).cloned()
    }

    /// A save of every part's current state: its last save when nothing changed since, else a
    /// new one. Parts that keep no saves are left out.
    pub async fn save_all(&self) -> Result<Saves, WorldError> {
        let mut out = Saves::new();
        for p in self.parts.iter().filter(|p| p.keeps_saves()) {
            let save = match self.reusable_save(p) {
                Some(s) => s,
                None => self.save_new(p).await?,
            };
            out.insert(p.name.clone(), save);
        }
        Ok(out)
    }

    /// Saves every part `names` names under that name.
    pub(super) async fn save_all_as(&self, names: &Saves) -> Result<(), WorldError> {
        for p in &self.parts {
            if let Some(save) = names.get(&p.name) {
                self.save_as(p, save).await?;
            }
        }
        Ok(())
    }

    /// Keys that differ between each part's live instance and `other[part]` (shared saves: base
    /// or gold), sorted; prefixed `part:` only when the world has more than one part. Parts not
    /// in the shared saves are skipped (see `unchecked_parts`).
    pub async fn diff_with(&self, other: &Saves) -> Result<Vec<String>, WorldError> {
        let many = self.parts.len() > 1;
        let mut out = vec![];
        for p in self.parts.iter().filter(|p| p.in_shared_saves()) {
            let save = other.get(&p.name).ok_or_else(|| {
                WorldError::other(format!(
                    "there is no save of the part {} to compare with",
                    p.name
                ))
            })?;
            let params = json!({"a": {"instance": p.instance}, "b": {"save": save}});
            // A malformed answer is an error, never "nothing differs".
            let d: Differing = call(&p.client, &p.label(), "diff", params).await?;
            out.extend(d.differing.into_iter().map(|k| {
                if many {
                    format!("{}:{k}", p.name)
                } else {
                    k
                }
            }));
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}
