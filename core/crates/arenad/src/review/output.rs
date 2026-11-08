//! What a reviewer must answer, and how its answer is checked: parsed as JSON, each field
//! validated, lists bounded, and every cited step (or run) checked against the record. A citation
//! the record doesn't have is dropped and said in `issues`; an item left with no citation it
//! started with is marked `unverified`.

use std::collections::BTreeSet;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_WRONG: usize = 8;
pub const MAX_SUGGESTIONS: usize = 8;
pub const MAX_WELL: usize = 6;
pub const MAX_DIFFERENCES: usize = 6;
/// Bytes of one text field the review keeps.
const MAX_TEXT: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Safety,
    Goal,
    Efficiency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Prompt,
    Team,
    Tool,
    Model,
    Process,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Better {
    A,
    B,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WentWrong {
    pub title: String,
    pub why: String,
    pub steps: Vec<i64>,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub change: String,
    pub why: String,
    pub kind: SuggestionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Text to add to the agent's prompt (prompt and team suggestions), for "Rewind and try this".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
    /// The step to rerun from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunReview {
    pub verdict: String,
    pub went_wrong: Vec<WentWrong>,
    pub suggestions: Vec<Suggestion>,
    pub went_well: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    pub title: String,
    pub a: String,
    pub b: String,
    pub why_it_matters: String,
    /// Steps of branch A (run comparison) or example runs (branch ids) of suite A.
    #[serde(default)]
    pub steps_a: Vec<Value>,
    #[serde(default)]
    pub steps_b: Vec<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareReview {
    pub headline: String,
    pub differences: Vec<Difference>,
    pub which_is_better: Better,
    pub why: String,
}

/// What a review may cite: the steps of a branch (or of the two branches compared), or the runs
/// of the two suites compared, and the agents of the team.
#[derive(Debug, Clone, Default)]
pub struct Citable {
    pub steps_a: BTreeSet<i64>,
    pub steps_b: BTreeSet<i64>,
    pub runs_a: BTreeSet<String>,
    pub runs_b: BTreeSet<String>,
    pub agents: BTreeSet<String>,
}

/// The JSON object in a model's answer: the answer itself, or inside a ```json fence, or the
/// outermost `{…}` of the text.
pub fn json_in(text: &str) -> Result<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return object(v);
    }
    let fenced = t
        .split("```")
        .nth(1)
        .map(|f| f.trim_start_matches("json").trim());
    if let Some(Ok(v)) = fenced.map(serde_json::from_str::<Value>) {
        return object(v);
    }
    match (t.find('{'), t.rfind('}')) {
        (Some(i), Some(j)) if i < j => object(serde_json::from_str(&t[i..=j])?),
        _ => bail!("the reviewer's answer has no JSON object"),
    }
}

fn object(v: Value) -> Result<Value> {
    match v {
        Value::Object(_) => Ok(v),
        _ => bail!("the reviewer's answer is JSON but not an object"),
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| super::text::cut(s, MAX_TEXT))
}

fn required(v: &Value, key: &str) -> Result<String> {
    text(v, key).ok_or_else(|| anyhow!("the review has no {key}"))
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// An enum field read leniently (case, spaces); `None` for a value outside it.
fn word<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Option<T> {
    let w = v
        .get(key)?
        .as_str()?
        .trim()
        .to_lowercase()
        .replace(' ', "_");
    serde_json::from_value(Value::String(w)).ok()
}

/// What checking found and changed, in plain words.
#[derive(Debug, Default)]
pub struct Issues(pub Vec<String>);

impl Issues {
    fn say(&mut self, s: String) {
        self.0.push(s);
    }
}

/// The cited steps of `v[key]` that `known` has; the others are said in `issues`. True in the
/// second place when some were cited and none was kept.
fn cited_steps(
    v: &Value,
    key: &str,
    known: &BTreeSet<i64>,
    issues: &mut Issues,
) -> (Vec<i64>, bool) {
    let cited = list(v, key);
    let mut kept = vec![];
    for c in cited {
        match c
            .as_i64()
            .or_else(|| c.as_str().and_then(|s| s.trim().parse().ok()))
        {
            Some(n) if known.contains(&n) => kept.push(n),
            _ => issues.say(format!("dropped citation {c}: no such step")),
        }
    }
    kept.dedup();
    let lost = !cited.is_empty() && kept.is_empty();
    (kept, lost)
}

fn went_wrong(v: &Value, c: &Citable, issues: &mut Issues) -> Option<WentWrong> {
    let (Some(title), Some(why)) = (text(v, "title"), text(v, "why")) else {
        issues.say("dropped an item without a title or why".into());
        return None;
    };
    let severity = word(v, "severity").unwrap_or_else(|| {
        issues.say(format!("{title}: unknown severity, read as goal"));
        Severity::Goal
    });
    let (steps, unverified) = cited_steps(v, "steps", &c.steps_a, issues);
    Some(WentWrong {
        title,
        why,
        steps,
        severity,
        unverified,
    })
}

fn suggestion(v: &Value, c: &Citable, issues: &mut Issues) -> Option<Suggestion> {
    let (Some(change), Some(why)) = (text(v, "change"), text(v, "why")) else {
        issues.say("dropped a suggestion without a change or why".into());
        return None;
    };
    let kind = word(v, "kind").unwrap_or(SuggestionKind::Process);
    let agent = text(v, "agent").filter(|a| {
        let known = c.agents.contains(a);
        if !known {
            issues.say(format!("dropped agent {a:?}: not in the team"));
        }
        known
    });
    let step = v.get("step").and_then(Value::as_i64).filter(|s| {
        let known = c.steps_a.contains(s);
        if !known {
            issues.say(format!("dropped citation {s}: no such step"));
        }
        known
    });
    Some(Suggestion {
        change,
        why,
        kind,
        agent,
        prompt_text: text(v, "prompt_text"),
        step,
    })
}

fn texts(v: &Value, key: &str, cap: usize) -> Vec<String> {
    let s = list(v, key).iter().filter_map(Value::as_str).map(str::trim);
    let s = s.filter(|s| !s.is_empty()).take(cap);
    s.map(|s| super::text::cut(s, MAX_TEXT)).collect()
}

/// A run review, checked against what the branch has.
pub fn run_review(v: &Value, c: &Citable) -> Result<(RunReview, Issues)> {
    let mut issues = Issues::default();
    let verdict = required(v, "verdict")?;
    let wrong = list(v, "went_wrong")
        .iter()
        .filter_map(|x| went_wrong(x, c, &mut issues));
    let went_wrong = wrong.take(MAX_WRONG).collect();
    let sugg = list(v, "suggestions")
        .iter()
        .filter_map(|x| suggestion(x, c, &mut issues));
    let suggestions = sugg.take(MAX_SUGGESTIONS).collect();
    let confidence = word::<Confidence>(v, "confidence").unwrap_or(Confidence::Medium);
    let review = RunReview {
        verdict,
        went_wrong,
        suggestions,
        went_well: texts(v, "went_well", MAX_WELL),
        confidence: serde_json::to_value(confidence)?
            .as_str()
            .unwrap_or("medium")
            .into(),
    };
    Ok((review, issues))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Confidence {
    High,
    Medium,
    Low,
}

/// What a comparison's sides cite: steps of two branches, or runs of two suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cites {
    Steps,
    Runs,
}

/// The citations of one side kept, as JSON values (steps as numbers, runs as ids).
fn cited_side(
    v: &Value,
    key: &str,
    cites: Cites,
    steps: &BTreeSet<i64>,
    runs: &BTreeSet<String>,
    issues: &mut Issues,
) -> (Vec<Value>, bool) {
    if cites == Cites::Steps {
        let (kept, lost) = cited_steps(v, key, steps, issues);
        return (kept.into_iter().map(Value::from).collect(), lost);
    }
    let cited = list(v, key);
    let mut kept = vec![];
    for c in cited {
        match c.as_str().map(str::trim) {
            Some(id) if runs.contains(id) => kept.push(Value::from(id)),
            _ => issues.say(format!("dropped citation {c}: no such run")),
        }
    }
    let lost = !cited.is_empty() && kept.is_empty();
    (kept, lost)
}

fn difference(v: &Value, c: &Citable, cites: Cites, issues: &mut Issues) -> Option<Difference> {
    let title = text(v, "title");
    let (Some(title), Some(why_it_matters)) = (title, text(v, "why_it_matters")) else {
        issues.say("dropped a difference without a title or why_it_matters".into());
        return None;
    };
    let side =
        |key, steps, runs, issues: &mut Issues| cited_side(v, key, cites, steps, runs, issues);
    let (steps_a, lost_a) = side("steps_a", &c.steps_a, &c.runs_a, issues);
    let (steps_b, lost_b) = side("steps_b", &c.steps_b, &c.runs_b, issues);
    Some(Difference {
        title,
        a: text(v, "a").unwrap_or_default(),
        b: text(v, "b").unwrap_or_default(),
        why_it_matters,
        unverified: (lost_a || lost_b) && steps_a.is_empty() && steps_b.is_empty(),
        steps_a,
        steps_b,
    })
}

/// A comparison review (of two branches, or two suites), checked against both sides.
pub fn compare_review(v: &Value, c: &Citable, cites: Cites) -> Result<(CompareReview, Issues)> {
    let mut issues = Issues::default();
    let headline = required(v, "headline")?;
    let diffs = list(v, "differences").iter();
    let diffs = diffs.filter_map(|x| difference(x, c, cites, &mut issues));
    let differences = diffs.take(MAX_DIFFERENCES).collect();
    let which_is_better = word(v, "which_is_better").unwrap_or_else(|| {
        issues.say("which_is_better missing or unknown, read as mixed".into());
        Better::Mixed
    });
    let review = CompareReview {
        headline,
        differences,
        which_is_better,
        why: text(v, "why").unwrap_or_default(),
    };
    Ok((review, issues))
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
