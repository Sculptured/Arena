//! The Bedrock Converse API and the canonical (OpenAI chat-completions) format, both ways. Pure.
//!
//! Request: system messages become `system`; user and assistant text become `text` blocks; an
//! assistant's tool calls become `toolUse` blocks and each tool message a `toolResult` block in a
//! user turn; turns of the same role in a row are merged (Converse wants them alternating);
//! `tools` become `toolConfig`; `max_tokens`, `temperature`, `top_p` and `stop` go in
//! `inferenceConfig`. Response: `text` blocks are the content, `toolUse` blocks the tool calls,
//! `stopReason` the finish reason, `usage` the token counts.

use serde_json::{json, Map, Value};

/// What stands in for an empty text, which Converse refuses.
const EMPTY: &str = "(empty)";

fn text_block(text: &str) -> Value {
    json!({"text": if text.is_empty() { EMPTY } else { text }})
}

/// A canonical message's content as text: a string, or the text of its parts.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn tool_use(call: &Value) -> Value {
    let f = &call["function"];
    let args = f["arguments"].as_str().unwrap_or("{}");
    let input: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
    json!({"toolUse": {"toolUseId": call["id"], "name": f["name"], "input": input}})
}

fn assistant_blocks(m: &Value) -> Vec<Value> {
    let text = content_text(&m["content"]);
    let calls = m["tool_calls"].as_array().map_or(&[][..], Vec::as_slice);
    let mut blocks: Vec<Value> = (!text.is_empty())
        .then(|| text_block(&text))
        .into_iter()
        .collect();
    blocks.extend(calls.iter().map(tool_use));
    if blocks.is_empty() {
        blocks.push(text_block(""));
    }
    blocks
}

fn tool_result(m: &Value) -> Value {
    let text = content_text(&m["content"]);
    json!({"toolResult": {"toolUseId": m["tool_call_id"], "content": [text_block(&text)]}})
}

/// Appends a turn, merged into the last one when it has the same role.
fn push_turn(turns: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = turns.last_mut().filter(|t| t["role"] == role) {
        let content = last["content"]
            .as_array_mut()
            .expect("a turn's content is a list");
        content.extend(blocks);
        return;
    }
    turns.push(json!({"role": role, "content": blocks}));
}

fn tool_config(tools: &[Value]) -> Value {
    let specs: Vec<Value> = tools
        .iter()
        .map(|t| {
            let f = &t["function"];
            let mut spec = json!({"name": f["name"], "inputSchema": {"json": f["parameters"]}});
            if let Some(d) = f["description"].as_str().filter(|d| !d.is_empty()) {
                spec["description"] = json!(d);
            }
            json!({"toolSpec": spec})
        })
        .collect();
    json!({"tools": specs})
}

fn inference_config(chat: &Value) -> Map<String, Value> {
    let mut c = Map::new();
    for (from, to) in [
        ("max_tokens", "maxTokens"),
        ("temperature", "temperature"),
        ("top_p", "topP"),
    ] {
        if let Some(v) = chat.get(from).filter(|v| v.is_number()) {
            c.insert(to.into(), v.clone());
        }
    }
    match &chat["stop"] {
        Value::String(s) => _ = c.insert("stopSequences".into(), json!([s])),
        Value::Array(a) => _ = c.insert("stopSequences".into(), json!(a)),
        _ => {}
    }
    c
}

/// The Converse request body for canonical request `chat` (the model goes in the URL).
pub(super) fn request(chat: &Value) -> Value {
    let messages = chat["messages"].as_array().map_or(&[][..], Vec::as_slice);
    let mut system = vec![];
    let mut turns = vec![];
    for m in messages {
        match m["role"].as_str().unwrap_or("") {
            "system" | "developer" => system.push(text_block(&content_text(&m["content"]))),
            "assistant" => push_turn(&mut turns, "assistant", assistant_blocks(m)),
            "tool" => push_turn(&mut turns, "user", vec![tool_result(m)]),
            _ => push_turn(
                &mut turns,
                "user",
                vec![text_block(&content_text(&m["content"]))],
            ),
        }
    }
    let mut body = json!({"messages": turns});
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if let Some(tools) = chat["tools"].as_array().filter(|t| !t.is_empty()) {
        body["toolConfig"] = tool_config(tools);
    }
    let config = inference_config(chat);
    if !config.is_empty() {
        body["inferenceConfig"] = Value::Object(config);
    }
    body
}

fn finish_reason(stop: Option<&str>) -> &'static str {
    match stop {
        Some("tool_use") => "tool_calls",
        Some("max_tokens" | "model_context_window_exceeded") => "length",
        Some("guardrail_intervened" | "content_filtered") => "content_filter",
        _ => "stop",
    }
}

fn tool_call(block: &Value) -> Value {
    let u = &block["toolUse"];
    json!({"id": u["toolUseId"], "type": "function",
           "function": {"name": u["name"], "arguments": u["input"].to_string()}})
}

fn usage(u: &Value) -> Value {
    let n = |k: &str| u[k].as_i64().unwrap_or(0);
    let prompt = n("inputTokens") + n("cacheReadInputTokens") + n("cacheWriteInputTokens");
    let completion = n("outputTokens");
    json!({"prompt_tokens": prompt, "completion_tokens": completion,
           "total_tokens": prompt + completion})
}

/// The canonical response for Converse answer `raw` to a call to `model`.
pub(super) fn response(raw: &Value, model: &str) -> Value {
    let blocks = raw["output"]["message"]["content"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let text: Vec<&str> = blocks.iter().filter_map(|b| b["text"].as_str()).collect();
    let calls: Vec<Value> = blocks
        .iter()
        .filter(|b| b.get("toolUse").is_some())
        .map(tool_call)
        .collect();
    let mut message = json!({"role": "assistant",
                             "content": if text.is_empty() { Value::Null } else { json!(text.join("\n")) }});
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": message,
                     "finish_reason": finish_reason(raw["stopReason"].as_str())}],
        "usage": usage(&raw["usage"]),
    })
}

/// A Converse error answer (`{"message": ...}`) in the canonical error shape.
pub(super) fn error(raw: &Value) -> Value {
    let message = [&raw["message"], &raw["Message"], &raw["raw"]]
        .into_iter()
        .find_map(Value::as_str)
        .unwrap_or("Bedrock gave no message");
    let mut e = json!({"message": message});
    if let Some(t) = raw["__type"].as_str() {
        e["type"] = json!(t.rsplit('#').next().unwrap_or(t));
    }
    json!({"error": e})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_loop_turns_into_alternating_converse_turns() {
        let chat = json!({"model": "m", "max_tokens": 512, "messages": [
            {"role": "system", "content": "be careful"},
            {"role": "user", "content": "ship order 7"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "t1", "type": "function", "function": {"name": "get_order", "arguments": "{\"id\":7}"}},
                {"id": "t2", "type": "function", "function": {"name": "stock", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "t1", "content": "{\"status\":\"open\"}"},
            {"role": "tool", "tool_call_id": "t2", "content": ""},
        ], "tools": [{"type": "function", "function": {"name": "get_order", "description": "",
                      "parameters": {"type": "object"}}}]});
        let body = request(&chat);
        assert_eq!(body["system"], json!([{"text": "be careful"}]));
        let turns = body["messages"].as_array().unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(
            turns[1]["content"][0]["toolUse"],
            json!({"toolUseId": "t1", "name": "get_order", "input": {"id": 7}})
        );
        // Both results in one user turn; an empty one still has text.
        assert_eq!(turns[2]["role"], "user");
        assert_eq!(
            turns[2]["content"][1]["toolResult"]["content"],
            json!([{"text": EMPTY}])
        );
        assert_eq!(
            body["toolConfig"]["tools"][0],
            json!({"toolSpec": {"name": "get_order", "inputSchema": {"json": {"type": "object"}}}})
        );
        assert_eq!(body["inferenceConfig"], json!({"maxTokens": 512}));
    }

    #[test]
    fn a_converse_answer_becomes_a_canonical_one() {
        let raw = json!({"output": {"message": {"role": "assistant", "content": [
            {"text": "Checking."},
            {"toolUse": {"toolUseId": "tu1", "name": "get_order", "input": {"id": 7}}}]}},
            "stopReason": "tool_use", "usage": {"inputTokens": 100, "outputTokens": 20, "totalTokens": 120}});
        let r = response(&raw, "anthropic.claude-x");
        let c = &r["choices"][0];
        assert_eq!(c["finish_reason"], "tool_calls");
        assert_eq!(c["message"]["content"], "Checking.");
        assert_eq!(
            c["message"]["tool_calls"][0]["function"],
            json!({"name": "get_order", "arguments": "{\"id\":7}"})
        );
        assert_eq!(
            r["usage"],
            json!({"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120})
        );
        let cut = response(&json!({"stopReason": "max_tokens"}), "m");
        assert_eq!(cut["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn an_error_keeps_its_message_and_type() {
        let e = error(
            &json!({"message": "Too many requests", "__type": "com.amazon#ThrottlingException"}),
        );
        assert_eq!(
            e,
            json!({"error": {"message": "Too many requests", "type": "ThrottlingException"}})
        );
    }
}
