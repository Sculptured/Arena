//! A run's scenario: the disruptions a run or suite asks for (`{disruptions: [{id, params}]}`),
//! expanded with the benchmark's templates (`describe.scenario_options`) and appended to the setup
//! args of the part each one names. The expanded scenario is recorded in `RunStart`; forks and
//! replays keep it because they restore or repeat the setup it changed.

use parts::proto::{
    DisruptionOption, ParamKind, ScenarioOptions, ScenarioParam, ScenarioRequest, SetupCall,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// One disruption chosen and expanded, as `RunStart` records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expanded {
    /// The option's id.
    pub id: String,
    pub label: String,
    /// The part whose setup call carries it.
    pub part: String,
    /// Every param's value, the defaults filled in.
    pub params: Map<String, Value>,
    /// The option's template with the params put in.
    pub disruption: Value,
}

/// A run's expanded scenario (empty: the task as it is).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub disruptions: Vec<Expanded>,
    /// The seed the run's world starts with, when the run asked for one (`POST /api/runs`
    /// `seed`); otherwise arenad picks one at random. `RunStart` records the seed either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Scenario {
    /// True when no disruption is chosen (a seed alone is recorded in `RunStart.seed`).
    pub fn is_empty(&self) -> bool {
        self.disruptions.is_empty()
    }
}

/// `req` expanded with `options`: each chosen option once, its params checked (a missing one
/// takes its default), and every expanded disruption's `id` different. The error says what is
/// wrong, for a 400.
pub fn expand(
    options: Option<&ScenarioOptions>,
    req: &ScenarioRequest,
) -> Result<Scenario, String> {
    if req.disruptions.is_empty() {
        return Ok(Scenario::default());
    }
    let options = options.ok_or("the benchmark offers no disruptions")?;
    let mut out = Scenario::default();
    for chosen in &req.disruptions {
        let opt = (options.disruptions.iter())
            .find(|o| o.id == chosen.id)
            .ok_or_else(|| format!("the benchmark offers no disruption {:?}", chosen.id))?;
        if out.disruptions.iter().any(|e| e.id == opt.id) {
            return Err(format!("disruption {:?} is chosen twice", opt.id));
        }
        out.disruptions.push(expand_one(opt, &chosen.params)?);
    }
    let ids: Vec<&Value> = out
        .disruptions
        .iter()
        .map(|e| &e.disruption["id"])
        .collect();
    if (1..ids.len()).any(|i| ids[..i].contains(&ids[i])) {
        return Err("two chosen disruptions expand to the same id".into());
    }
    Ok(out)
}

fn expand_one(opt: &DisruptionOption, given: &Map<String, Value>) -> Result<Expanded, String> {
    if let Some(unknown) = given
        .keys()
        .find(|k| !opt.params.iter().any(|p| &p.name == *k))
    {
        return Err(format!("disruption {:?} has no param {unknown:?}", opt.id));
    }
    let mut params = Map::new();
    for p in &opt.params {
        let value = given.get(&p.name).unwrap_or(&p.default);
        check_param(p, value).map_err(|e| format!("disruption {:?}: {e}", opt.id))?;
        params.insert(p.name.clone(), value.clone());
    }
    Ok(Expanded {
        id: opt.id.clone(),
        label: opt.label.clone(),
        part: opt.part.clone(),
        disruption: fill(&opt.template, &params),
        params,
    })
}

/// Why `value` doesn't suit the param, if it doesn't.
fn check_param(p: &ScenarioParam, value: &Value) -> Result<(), String> {
    match p.kind {
        ParamKind::Int => {
            let n = value
                .as_i64()
                .ok_or_else(|| format!("{} must be a whole number", p.name))?;
            let low = p.min.is_some_and(|m| n < m);
            let high = p.max.is_some_and(|m| n > m);
            match low || high {
                true => Err(format!(
                    "{} must be from {:?} to {:?}",
                    p.name, p.min, p.max
                )),
                false => Ok(()),
            }
        }
        ParamKind::Choice => match p.choices.as_ref().is_some_and(|c| c.contains(value)) {
            true => Ok(()),
            false => Err(format!("{} must be one of {:?}", p.name, p.choices)),
        },
    }
}

/// `template` with `{param}` put in its strings: a string that is exactly one placeholder
/// becomes the param's value as it is; elsewhere the value's text replaces it.
pub fn fill(template: &Value, params: &Map<String, Value>) -> Value {
    match template {
        Value::String(s) => fill_text(s, params),
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, params)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), fill(v, params)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn fill_text(s: &str, params: &Map<String, Value>) -> Value {
    let whole = s.strip_prefix('{').and_then(|r| r.strip_suffix('}'));
    if let Some(v) = whole.and_then(|name| params.get(name)) {
        return v.clone();
    }
    let mut out = s.to_string();
    for (name, v) in params {
        let text = v.as_str().map_or_else(|| v.to_string(), str::to_string);
        out = out.replace(&format!("{{{name}}}"), &text);
    }
    Value::String(out)
}

/// The setup calls of a task (`setup`) with the scenario's disruptions appended to the `args.
/// disruptions` of each part's call: the call of the tool `options.setup` names for the part,
/// else its last call, else a new call from `options.setup`.
pub fn setup_calls(
    setup: &[SetupCall],
    scenario: &Scenario,
    options: Option<&ScenarioOptions>,
) -> Result<Vec<SetupCall>, String> {
    let mut calls = setup.to_vec();
    for e in &scenario.disruptions {
        let i = match call_of(&calls, &e.part, options) {
            Some(i) => i,
            None => {
                calls.push(new_call(&e.part, options)?);
                calls.len() - 1
            }
        };
        append(&mut calls[i].args, &e.disruption, &e.part)?;
    }
    Ok(calls)
}

/// The index of the call of `part` that carries its disruptions.
fn call_of(calls: &[SetupCall], part: &str, options: Option<&ScenarioOptions>) -> Option<usize> {
    let tool = options.and_then(|o| o.setup.get(part)).map(|t| &t.tool);
    let of_part = |c: &SetupCall| c.part == part;
    let matching = calls
        .iter()
        .position(|c| of_part(c) && tool.is_some_and(|t| &c.tool == t));
    matching.or_else(|| calls.iter().rposition(of_part))
}

fn new_call(part: &str, options: Option<&ScenarioOptions>) -> Result<SetupCall, String> {
    let template = options.and_then(|o| o.setup.get(part)).ok_or_else(|| {
        format!("the task has no setup call of the part {part} to carry its disruptions")
    })?;
    Ok(SetupCall {
        part: part.into(),
        tool: template.tool.clone(),
        args: template.args.clone(),
    })
}

fn append(args: &mut Value, disruption: &Value, part: &str) -> Result<(), String> {
    if args.is_null() {
        *args = json!({});
    }
    let map = args
        .as_object_mut()
        .ok_or_else(|| format!("the setup call of {part} takes no object to add disruptions to"))?;
    let list = map.entry("disruptions").or_insert_with(|| json!([]));
    let list = list
        .as_array_mut()
        .ok_or_else(|| format!("the setup call of {part} has disruptions that are not a list"))?;
    list.push(disruption.clone());
    Ok(())
}

#[cfg(test)]
mod tests;
