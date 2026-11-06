//! What a world tells monitoring views about itself: the kind of system each part is (its
//! integration's `describe.system`) and, per tool, whether it reads or writes and which part it
//! runs on. Recorded in a run's `RunStart`, so a UI can say which system each step touched from
//! the run's record alone; runs recorded before this have neither (unknown, never guessed).

use serde_json::{json, Map, Value};

use super::{Route, World};

impl World {
    /// `{part: {kind, product, stands_in_for?, simulated}}` of the parts whose integration says
    /// what kind of system it is; a part that does not say is absent.
    pub fn systems(&self) -> Map<String, Value> {
        self.parts
            .iter()
            .filter_map(|p| Some((p.name.clone(), json!(p.describe.system.as_ref()?))))
            .collect()
    }

    /// `{tool: {effect, part?}}` of every tool agents get from the world. A benchmark's own tool
    /// has no fixed part (its call says which part it ran on).
    pub fn tool_info(&self) -> Map<String, Value> {
        self.tools
            .iter()
            .map(|(t, route)| {
                let mut info = json!({"effect": t.effect});
                if let Route::Part(i) = route {
                    info["part"] = json!(self.parts.get(*i).map(|p| &p.name));
                }
                (t.name.clone(), info)
            })
            .collect()
    }
}
