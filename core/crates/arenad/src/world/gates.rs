//! A world's gates: a tool an agent calls goes through only if a document of another part allows
//! it. Declared by the benchmark and read with `inspect`, so neither part knows of the other.

use parts::proto::{Docs, Gate, ToolOutput};
use serde_json::{json, Value};

use super::{call, World, WorldError};

/// `template` with each `{arg}` replaced by the call's argument; an error names what is missing.
fn fill(template: &str, args: &Value) -> Result<String, String> {
    let (mut out, mut rest) = (String::new(), template);
    while let Some((before, after)) = rest.split_once('{') {
        let (name, tail) = after
            .split_once('}')
            .ok_or_else(|| format!("the gate's key {template} has an unclosed brace"))?;
        let value = args
            .get(name)
            .filter(|v| !v.is_null())
            .ok_or_else(|| format!("this call needs the argument {name}"))?;
        out.push_str(before);
        match value.as_str() {
            Some(s) => out.push_str(s),
            None => out.push_str(&value.to_string()),
        }
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// The value a `require` entry asks for: the call's argument for the string `"{arg}"`, else itself.
fn wanted<'a>(spec: &'a Value, args: &'a Value) -> Option<&'a Value> {
    let name = spec
        .as_str()
        .and_then(|s| s.strip_prefix('{'))
        .and_then(|s| s.strip_suffix('}'));
    match name {
        Some(name) => args.get(name),
        None => Some(spec),
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 0.005,
        _ => a == b,
    }
}

/// Why `doc` doesn't allow the call with `args`, if it doesn't.
fn not_allowed(gate: &Gate, key: &str, doc: &Value, args: &Value) -> Option<String> {
    if doc.is_null() {
        return Some(format!("there is no {key} in {}", gate.part));
    }
    gate.require.iter().find_map(|(pointer, spec)| {
        let want = wanted(spec, args).unwrap_or(&Value::Null);
        let got = doc.pointer(pointer).unwrap_or(&Value::Null);
        (!same(got, want)).then(|| format!("{key}{pointer} is {got}, and this call needs {want}"))
    })
}

fn refusal(gate: &Gate, why: &str, reads: Vec<String>) -> ToolOutput {
    ToolOutput {
        error: Some(format!("{} refused: {why}", gate.tool)),
        reads,
        ..ToolOutput::default()
    }
}

impl World {
    /// What `tool` answers instead of running when a gate holds it back. A part that has stopped
    /// is the only failure; a read that fails holds the call back.
    pub(super) async fn gate_refusal(
        &self,
        tool: &str,
        args: &Value,
    ) -> Result<Option<ToolOutput>, WorldError> {
        for gate in self.gates.iter().filter(|g| g.tool == tool) {
            if let Some(out) = self.check_gate(gate, args).await? {
                return Ok(Some(out));
            }
        }
        Ok(None)
    }

    async fn check_gate(
        &self,
        gate: &Gate,
        args: &Value,
    ) -> Result<Option<ToolOutput>, WorldError> {
        let key = match fill(&gate.key, args) {
            Ok(key) => key,
            Err(why) => return Ok(Some(refusal(gate, &why, vec![]))),
        };
        let p = self.part(&gate.part)?;
        let params = json!({"target": {"instance": p.instance}, "keys": [key]});
        let read: Result<Docs, WorldError> = call(&p.client, &p.label(), "inspect", params).await;
        let docs = match read {
            Ok(docs) => docs,
            Err(e) if e.fatal => return Err(e),
            Err(e) => return Ok(Some(refusal(gate, &e.message, vec![]))),
        };
        let doc = docs.docs.get(&key).unwrap_or(&Value::Null);
        let why = not_allowed(gate, &key, doc, args);
        let scope = format!("{}:{key}", gate.part);
        Ok(why.map(|why| refusal(gate, &why, vec![scope])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(require: Value) -> Gate {
        Gate {
            tool: "book".into(),
            part: "approvals".into(),
            key: "change/{id}".into(),
            require: require.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn keys_are_filled_from_the_calls_arguments() {
        assert_eq!(
            fill("change/{id}", &json!({"id": "c1"})).unwrap(),
            "change/c1"
        );
        assert_eq!(fill("a/{n}/b", &json!({"n": 3})).unwrap(), "a/3/b");
        assert_eq!(fill("plain", &json!({})).unwrap(), "plain");
        assert!(fill("change/{id}", &json!({}))
            .unwrap_err()
            .contains("argument id"));
        assert!(fill("change/{id", &json!({"id": 1})).is_err());
    }

    #[test]
    fn a_document_allows_a_call_only_when_every_value_matches() {
        let g = gate(json!({"/decision/approved": true, "/payload/amount": "{amount}"}));
        let doc = json!({"decision": {"approved": true}, "payload": {"amount": 640}});
        let args = json!({"amount": 640.0});
        assert_eq!(not_allowed(&g, "k", &doc, &args), None);
        let e = not_allowed(&g, "k", &doc, &json!({"amount": 700})).unwrap();
        assert!(e.contains("/payload/amount is 640"), "{e}");
        let waiting = json!({"decision": null, "payload": {"amount": 640}});
        assert!(not_allowed(&g, "k", &waiting, &args).is_some());
        assert!(not_allowed(&g, "k", &Value::Null, &args)
            .unwrap()
            .contains("no k"));
    }
}
