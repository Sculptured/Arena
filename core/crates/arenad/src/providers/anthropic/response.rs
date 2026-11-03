//! A Claude Messages API response as a canonical (OpenAI chat-completions) response.
//!
//! - `text` blocks become the message's content, `tool_use` blocks its `tool_calls`;
//! - every block (thinking included, which may have empty text) is kept as it came on the
//!   message, for the next request of the conversation to send back;
//! - `stop_reason` becomes `finish_reason`, and `usage` the canonical token counts.

use serde_json::{json, Value};

use super::{CONTENT_KEY, CONTEXT_KEY};

/// The canonical response for Messages API message `raw`, answered to a request made under
/// `context` (see [`super::request::context_of`]).
pub(super) fn translate(raw: &Value, context: &str) -> Value {
    let blocks = raw["content"].as_array().map_or(&[][..], Vec::as_slice);
    let finish = finish_reason(raw["stop_reason"].as_str());
    let mut choice = json!({
        "index": 0,
        "message": message(blocks, finish, context),
        "finish_reason": finish,
    });
    // Why a turn was declined (`refusal`): its category, and what the API explains.
    if let Some(details) = raw.get("stop_details").filter(|d| !d.is_null()) {
        choice["stop_details"] = details.clone();
    }
    json!({
        "id": raw["id"],
        "object": "chat.completion",
        "model": raw["model"],
        "choices": [choice],
        "usage": usage(&raw["usage"]),
    })
}

fn finish_reason(stop: Option<&str>) -> &'static str {
    match stop {
        Some("tool_use") => "tool_calls",
        Some("max_tokens" | "model_context_window_exceeded") => "length",
        Some("refusal") => "content_filter",
        // `end_turn`, `stop_sequence`, and `pause_turn`, which only server tools (never sent
        // here) give.
        _ => "stop",
    }
}

fn message(blocks: &[Value], finish: &str, context: &str) -> Value {
    let mut m = json!({"role": "assistant", "content": text_of(blocks)});
    let kept = kept_blocks(blocks, finish);
    let calls: Vec<Value> = kept
        .iter()
        .filter(|b| b["type"] == "tool_use")
        .map(tool_call)
        .collect();
    if !calls.is_empty() {
        m["tool_calls"] = json!(calls);
    }
    m[CONTENT_KEY] = json!(kept);
    m[CONTEXT_KEY] = json!(context);
    m
}

/// The text blocks' text, `null` when there is none.
fn text_of(blocks: &[Value]) -> Value {
    let text: String = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        Value::Null
    } else {
        json!(text)
    }
}

/// The blocks to offer and send back. A turn cut off or declined may hold a tool call with
/// partial input: it is neither offered nor kept (a `tool_use` with no `tool_result` after it
/// would be refused when sent back).
fn kept_blocks(blocks: &[Value], finish: &str) -> Vec<Value> {
    let offered = !matches!(finish, "length" | "content_filter");
    blocks
        .iter()
        .filter(|b| offered || b["type"] != "tool_use")
        .cloned()
        .collect()
}

fn tool_call(block: &Value) -> Value {
    json!({
        "id": block["id"],
        "type": "function",
        "function": {"name": block["name"], "arguments": block["input"].to_string()},
    })
}

/// Prompt tokens count the cached input too: read from the cache, and written to it.
fn usage(u: &Value) -> Value {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    let prompt =
        n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    let completion = n("output_tokens");
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(content: Value, stop: &str) -> Value {
        json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
               "content": content, "stop_reason": stop,
               "usage": {"input_tokens": 100, "output_tokens": 20,
                         "cache_read_input_tokens": 30, "cache_creation_input_tokens": 5}})
    }

    fn thinking() -> Value {
        json!({"type": "thinking", "thinking": "", "signature": "sig"})
    }

    #[test]
    fn tool_use_becomes_tool_calls_and_the_blocks_are_kept() {
        let blocks = json!([
            thinking(),
            {"type": "text", "text": "Checking."},
            {"type": "tool_use", "id": "t1", "name": "get", "input": {"id": "a"}},
            {"type": "tool_use", "id": "t2", "name": "get", "input": {}},
        ]);
        let v = translate(&raw(blocks.clone(), "tool_use"), "ctx");
        let choice = &v["choices"][0];
        assert_eq!(choice["finish_reason"], "tool_calls");
        let m = &choice["message"];
        assert_eq!(
            (&m["role"], &m["content"]),
            (&json!("assistant"), &json!("Checking."))
        );
        let calls = m["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            json!({"id": "t1", "type": "function",
                   "function": {"name": "get", "arguments": r#"{"id":"a"}"#}})
        );
        assert_eq!((&m[CONTENT_KEY], &m[CONTEXT_KEY]), (&blocks, &json!("ctx")));
        assert_eq!(
            (&v["id"], &v["model"]),
            (&json!("msg_1"), &json!("claude-opus-5-5"))
        );
    }

    #[test]
    fn usage_counts_cached_input_as_prompt_tokens() {
        let v = translate(&raw(json!([]), "end_turn"), "c");
        assert_eq!(
            v["usage"],
            json!({"prompt_tokens": 135, "completion_tokens": 20, "total_tokens": 155})
        );
        let bare = json!({"type": "message", "content": [], "usage": {"input_tokens": 3, "output_tokens": 1}});
        assert_eq!(translate(&bare, "c")["usage"]["total_tokens"], 4);
    }

    #[test]
    fn stop_reasons_map_to_finish_reasons() {
        for (stop, finish) in [
            ("end_turn", "stop"),
            ("stop_sequence", "stop"),
            ("pause_turn", "stop"),
            ("tool_use", "tool_calls"),
            ("max_tokens", "length"),
            ("refusal", "content_filter"),
        ] {
            let v = translate(&raw(json!([{"type": "text", "text": "x"}]), stop), "c");
            assert_eq!(v["choices"][0]["finish_reason"], finish, "{stop}");
        }
    }

    #[test]
    fn a_thinking_only_answer_has_no_content() {
        let v = translate(&raw(json!([thinking()]), "end_turn"), "c");
        let m = &v["choices"][0]["message"];
        assert_eq!((&m["content"], m.get("tool_calls")), (&Value::Null, None));
        assert_eq!(m[CONTENT_KEY], json!([thinking()]));
    }

    #[test]
    fn refusals_and_cut_offs_offer_no_tool_calls() {
        let partial =
            json!([{"type": "tool_use", "id": "t1", "name": "get", "input": {"id": "a"}}]);
        let mut refused = raw(partial.clone(), "refusal");
        refused["stop_details"] = json!({"type": "refusal", "category": "cyber"});
        let v = translate(&refused, "c");
        let choice = &v["choices"][0];
        assert_eq!(choice["finish_reason"], "content_filter");
        assert_eq!(choice["stop_details"]["category"], "cyber");
        assert_eq!(choice["message"].get("tool_calls"), None);
        assert_eq!(choice["message"][CONTENT_KEY], json!([]));
        let cut = translate(&raw(partial, "max_tokens"), "c");
        assert_eq!(cut["choices"][0]["message"].get("tool_calls"), None);
        assert_eq!(cut["choices"][0].get("stop_details"), None);
    }
}
