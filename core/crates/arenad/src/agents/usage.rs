//! Counts one Claude Code session's model calls from its stream-json, as reports arenad records
//! as `model_call` events (the shape an external system reports in, `external::UsageReport`).
//!
//! Two things in that stream need care:
//! - Claude Code repeats an assistant message once per content block (same message id, same
//!   usage), so a call is reported once, by its id.
//! - The `output_tokens` of a streamed assistant message is a placeholder; the real count is in
//!   the turn's `result` usage, so the turn's last call is reported again with the output the
//!   other calls don't account for (a report under the same id replaces the earlier one).
//! - Some calls are never streamed (compacting the context): what the session's cumulative
//!   `modelUsage` has beyond the streamed calls is reported as one more call.

use std::collections::HashSet;

use serde_json::Value;

use crate::external::UsageReport;

#[derive(Default)]
pub struct UsageMeter {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    seen: HashSet<String>,
    turn: Vec<UsageReport>,
    turns: usize,
}

fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn call_report(message: &Value) -> UsageReport {
    let u = &message["usage"];
    UsageReport {
        message_id: message["id"].as_str().map(str::to_string),
        model: message["model"].as_str().map(str::to_string),
        input_tokens: n(u, "input_tokens"),
        cache_creation_input_tokens: n(u, "cache_creation_input_tokens"),
        cache_read_input_tokens: n(u, "cache_read_input_tokens"),
        output_tokens: n(u, "output_tokens"),
        context_window: None,
        compaction: None,
    }
}

fn context(r: &UsageReport) -> i64 {
    r.input_tokens + r.cache_creation_input_tokens + r.cache_read_input_tokens
}

impl UsageMeter {
    /// An assistant message: its call's report, the first time its id is seen.
    pub fn assistant(&mut self, message: &Value) -> Option<UsageReport> {
        let r = call_report(message);
        let id = r.message_id.clone()?;
        if !self.seen.insert(id) {
            return None;
        }
        self.prompt_tokens += context(&r);
        self.completion_tokens += r.output_tokens;
        self.turn.push(r.clone());
        Some(r)
    }

    /// The turn's `result`: the reports that correct its calls (see the module's notes).
    pub fn turn_ended(&mut self, result: &Value) -> Vec<UsageReport> {
        let calls = std::mem::take(&mut self.turn);
        self.turns += 1;
        let mut out = vec![];
        if let Some(total_out) = result["usage"]["output_tokens"].as_i64() {
            out.extend(self.give_output(&calls, total_out));
        }
        out.extend(self.unstreamed(&result["modelUsage"]));
        out
    }

    fn give_output(&mut self, calls: &[UsageReport], total_out: i64) -> Option<UsageReport> {
        let (last, rest) = calls.split_last()?;
        let others: i64 = rest.iter().map(|c| c.output_tokens).sum();
        let corrected = (total_out - others).max(0);
        if corrected == last.output_tokens {
            return None;
        }
        self.completion_tokens += corrected - last.output_tokens;
        Some(UsageReport {
            output_tokens: corrected,
            ..last.clone()
        })
    }

    /// The session's billed tokens (cumulative, every model) beyond the streamed calls' sums.
    fn unstreamed(&mut self, model_usage: &Value) -> Option<UsageReport> {
        let models = model_usage.as_object().filter(|m| !m.is_empty())?;
        let total = |keys: &[&str]| -> i64 {
            let each = models
                .values()
                .map(|u| keys.iter().map(|k| n(u, k)).sum::<i64>());
            each.sum()
        };
        let input = [
            "inputTokens",
            "cacheReadInputTokens",
            "cacheCreationInputTokens",
        ];
        let gap_in = total(&input) - self.prompt_tokens;
        let gap_out = total(&["outputTokens"]) - self.completion_tokens;
        if gap_in <= 0 && gap_out <= 0 {
            return None;
        }
        let (gap_in, gap_out) = (gap_in.max(0), gap_out.max(0));
        self.prompt_tokens += gap_in;
        self.completion_tokens += gap_out;
        Some(UsageReport {
            message_id: Some(format!("unstreamed-{}", self.turns)),
            model: models.keys().next().cloned(),
            input_tokens: gap_in,
            output_tokens: gap_out,
            ..UsageReport::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(id: &str, input: i64, cache: i64, out: i64) -> Value {
        json!({"id": id, "model": "m", "usage": {"input_tokens": input,
               "cache_read_input_tokens": cache, "output_tokens": out}})
    }

    #[test]
    fn a_call_counts_once_and_the_turn_gives_it_its_real_output() {
        let mut m = UsageMeter::default();
        assert!(m.assistant(&msg("a", 10, 90, 1)).is_some());
        assert!(
            m.assistant(&msg("a", 10, 90, 1)).is_none(),
            "repeated per block"
        );
        assert!(m.assistant(&msg("b", 5, 100, 2)).is_some());
        let usage =
            json!({"m": {"inputTokens": 15, "cacheReadInputTokens": 190, "outputTokens": 40}});
        let fixes = m.turn_ended(&json!({"usage": {"output_tokens": 40}, "modelUsage": usage}));
        assert_eq!(fixes.len(), 1, "the streamed calls account for everything");
        assert_eq!(fixes[0].message_id.as_deref(), Some("b"));
        assert_eq!(fixes[0].output_tokens, 39);
        assert_eq!((m.prompt_tokens, m.completion_tokens), (205, 40));
        let usage =
            json!({"m": {"inputTokens": 1015, "cacheReadInputTokens": 190, "outputTokens": 60}});
        let fixes = m.turn_ended(&json!({"usage": {}, "modelUsage": usage}));
        assert_eq!(fixes[0].message_id.as_deref(), Some("unstreamed-2"));
        assert_eq!((fixes[0].input_tokens, fixes[0].output_tokens), (1000, 20));
    }
}
