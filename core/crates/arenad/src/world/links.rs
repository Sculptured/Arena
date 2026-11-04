//! A world's links: which calls between parts an event sets off, and marking what they wrote so
//! the step that follows saves it.

use parts::proto::Link;
use serde_json::{Map, Value};

use super::World;

/// The args of a call `link` makes for an event with `data`: a string starting with `/` is a JSON
/// pointer into `data`, anything else a constant. An argument the event lacks is an error.
pub fn link_args(link: &Link, data: &Value) -> Result<Value, String> {
    let mut args = Map::new();
    for (name, spec) in &link.args {
        let value = match spec.as_str().filter(|s| s.starts_with('/')) {
            Some(pointer) => data.pointer(pointer).cloned().ok_or_else(|| {
                format!(
                    "the {} event of {} has no {pointer} for the argument {name}",
                    link.event, link.from
                )
            })?,
            None => spec.clone(),
        };
        args.insert(name.clone(), value);
    }
    Ok(Value::Object(args))
}

impl World {
    /// The links an event of kind `kind` from `part` sets off, in the order the benchmark names
    /// them.
    pub fn links_from(&self, part: &str, kind: &str) -> Vec<Link> {
        let from = |l: &&Link| l.from == part && l.event == kind;
        self.links.iter().filter(from).cloned().collect()
    }

    /// Remembers that a link's call wrote to `part`, so the next save of what changed includes it.
    pub fn mark_written(&self, part: &str) {
        let mut track = self.track.lock().unwrap();
        track.changed.insert(part.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn link(args: Value) -> Link {
        Link {
            from: "hub".into(),
            event: "booked".into(),
            to: "plant".into(),
            tool: "add".into(),
            args: args.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn args_come_from_the_events_data_or_are_constants() {
        let l = link(json!({"part": "/detail/part", "qty": 1, "note": "plain"}));
        let data = json!({"detail": {"part": "SP-1"}});
        let want = json!({"part": "SP-1", "qty": 1, "note": "plain"});
        assert_eq!(link_args(&l, &data), Ok(want));
    }

    #[test]
    fn an_argument_the_event_lacks_is_an_error() {
        let e = link_args(&link(json!({"part": "/part"})), &json!({})).unwrap_err();
        assert!(e.contains("no /part for the argument part"), "{e}");
    }
}
