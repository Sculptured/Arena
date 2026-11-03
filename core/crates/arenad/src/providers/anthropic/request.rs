//! A canonical (OpenAI chat-completions) request as a Claude Messages API request.
//!
//! - `system` and `developer` messages become the one `system` prompt;
//! - `tools[].function` become `tools` with an `input_schema`;
//! - assistant `tool_calls` become `tool_use` blocks, and `tool` messages `tool_result` blocks,
//!   the results of one turn's calls together in one user turn;
//! - an assistant message that carries Claude's own blocks sends them back unchanged.

use serde_json::{json, Value};

use super::{CONTENT_KEY, CONTEXT_KEY};

/// The Messages API request for canonical `chat` (with `max_tokens` when it names none), and
/// the context its thinking will be bound to (see [`context_of`]). Only the fields below are
/// carried over: on API version 2026-10-21, sending `temperature`, `top_p` or `top_k` is a 400
/// on every model, so those (and the other chat-completions sampling fields) are dropped.
pub(super) fn translate(chat: &Value, max_tokens: u64) -> (Value, String) {
    let max_tokens = named_max_tokens(chat).unwrap_or(max_tokens);
    let mut out = json!({"model": chat["model"], "max_tokens": max_tokens});
    if let Some(system) = system(chat) {
        out["system"] = json!(system);
    }
    if let Some(tools) = tools(chat) {
        out["tools"] = tools;
    }
    if let Some(choice) = tool_choice(&chat["tool_choice"]) {
        out["tool_choice"] = choice;
    }
    let context = context_of(&out);
    out["messages"] = Value::Array(turns(chat, &context));
    (out, context)
}

/// A fingerprint of what a thinking block is bound to besides the messages before it: the
/// system prompt and the tools of `request`, whatever their order (FNV-1a over their JSON,
/// stable across builds).
fn context_of(request: &Value) -> String {
    let mut tools = request["tools"].as_array().cloned().unwrap_or_default();
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let bound = json!([request.get("system"), tools]).to_string();
    let hash = bound.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

fn named_max_tokens(chat: &Value) -> Option<u64> {
    ["max_completion_tokens", "max_tokens"]
        .iter()
        .find_map(|k| chat[*k].as_u64())
}

fn messages(chat: &Value) -> impl Iterator<Item = &Value> {
    chat["messages"].as_array().into_iter().flatten()
}

fn is_system(m: &Value) -> bool {
    matches!(m["role"].as_str(), Some("system" | "developer"))
}

/// The text of a message's content: a string, or its parts' text (see [`part_text`]).
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(part_text).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A content part's text; a part that isn't text (an image, audio, a file) is not sent, and a
/// placeholder says so where it was, so its absence shows.
fn part_text(part: &Value) -> String {
    if let Some(text) = part["text"].as_str() {
        return text.to_string();
    }
    let kind = match part["type"].as_str() {
        Some("image_url" | "image") => "image",
        Some("input_audio") => "audio",
        Some(other) => other,
        None => "content",
    };
    format!("[{kind} omitted: Arena sends text only]")
}

/// The system and developer messages, in order, as one prompt.
fn system(chat: &Value) -> Option<String> {
    let parts: Vec<String> = messages(chat)
        .filter(|m| is_system(m))
        .map(|m| text_of(&m["content"]))
        .filter(|t| !t.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

fn tools(chat: &Value) -> Option<Value> {
    let tools: Vec<Value> = chat["tools"]
        .as_array()?
        .iter()
        .filter_map(|t| tool(&t["function"]))
        .collect();
    (!tools.is_empty()).then_some(Value::Array(tools))
}

fn tool(f: &Value) -> Option<Value> {
    let schema = match &f["parameters"] {
        p @ Value::Object(_) => p.clone(),
        _ => json!({"type": "object", "properties": {}}),
    };
    let mut t = json!({"name": f["name"].as_str()?, "input_schema": schema});
    if let Some(d) = f["description"].as_str() {
        t["description"] = json!(d);
    }
    Some(t)
}

/// Forced tool use (`any`, `tool`) is a 400 on current models, so `"required"` and a named
/// function become `auto` (the prompt is what asks for the call); `"none"` stays `none`.
fn tool_choice(choice: &Value) -> Option<Value> {
    let kind = match choice {
        Value::Null => return None,
        Value::String(s) if s == "none" => "none",
        _ => "auto",
    };
    Some(json!({"type": kind}))
}

/// The conversation as alternating user and assistant turns of content blocks.
fn turns(chat: &Value, context: &str) -> Vec<Value> {
    let mut out = vec![];
    for m in messages(chat).filter(|m| !is_system(m)) {
        let (role, blocks) = match m["role"].as_str() {
            Some("assistant") => ("assistant", assistant_blocks(m, context)),
            Some("tool") => ("user", vec![tool_result(m)]),
            _ => ("user", user_blocks(&m["content"])),
        };
        push_turn(&mut out, role, blocks);
    }
    out
}

/// Appends `blocks` as a `role` turn, into the last turn when it has the same role: turns
/// alternate, and the results of parallel tool calls go back in one user turn.
fn push_turn(turns: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = turns.last_mut().filter(|t| t["role"] == role) {
        if let Some(content) = last["content"].as_array_mut() {
            content.extend(blocks);
            return;
        }
    }
    turns.push(json!({"role": role, "content": blocks}));
}

fn text_block(text: &str) -> Option<Value> {
    // The API refuses empty and whitespace-only text blocks.
    (!text.trim().is_empty()).then(|| json!({"type": "text", "text": text}))
}

/// A user message's text, as text blocks (one per part, see [`part_text`]).
fn user_blocks(content: &Value) -> Vec<Value> {
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| text_block(&part_text(p)))
            .collect(),
        other => text_block(&text_of(other)).into_iter().collect(),
    }
}

/// A tool message as a `tool_result`, marked `is_error` when the tool answered `{"error": …}`.
fn tool_result(m: &Value) -> Value {
    let mut block = json!({"type": "tool_result", "tool_use_id": m["tool_call_id"]});
    let content = text_of(&m["content"]);
    if is_error(&content) {
        block["is_error"] = json!(true);
    }
    if !content.is_empty() {
        block["content"] = json!(content);
    }
    block
}

fn is_error(content: &str) -> bool {
    serde_json::from_str::<Value>(content).is_ok_and(|v| v.get("error").is_some())
}

/// Claude's own blocks when the message carries them, unchanged; without their thinking when
/// they were made under another system prompt or tool set (a fork that changed the prompt),
/// where the API would refuse it. Else the blocks rebuilt from the canonical message.
fn assistant_blocks(m: &Value, context: &str) -> Vec<Value> {
    match m[CONTENT_KEY].as_array() {
        Some(blocks) if m[CONTEXT_KEY] == context => blocks.clone(),
        Some(blocks) => blocks.iter().filter(|b| !is_thinking(b)).cloned().collect(),
        None => rebuilt(m),
    }
}

fn is_thinking(block: &Value) -> bool {
    matches!(
        block["type"].as_str(),
        Some("thinking" | "redacted_thinking")
    )
}

fn rebuilt(m: &Value) -> Vec<Value> {
    let calls = m["tool_calls"].as_array().into_iter().flatten();
    text_block(&text_of(&m["content"]))
        .into_iter()
        .chain(calls.map(tool_use))
        .collect()
}

fn tool_use(call: &Value) -> Value {
    let f = &call["function"];
    let input = arguments(f["arguments"].as_str().unwrap_or(""));
    json!({"type": "tool_use", "id": call["id"], "name": f["name"], "input": input})
}

/// The parsed arguments; `{}` when they aren't a JSON object (their raw text stays in the
/// recorded canonical request).
fn arguments(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, name: &str, args: &str) -> Value {
        json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})
    }

    fn tool_msg(id: &str, content: &str) -> Value {
        json!({"role": "tool", "tool_call_id": id, "content": content})
    }

    const BUDGET: u64 = 16_000;

    /// The request for `chat`, with the default budget.
    fn body(chat: &Value) -> Value {
        translate(chat, BUDGET).0
    }

    fn chat(messages: Value) -> Value {
        json!({"model": "claude-opus-5-5", "messages": messages})
    }

    #[test]
    fn system_messages_merge_and_max_tokens_defaults() {
        let req = body(&chat(json!([
            {"role": "system", "content": "You are a."},
            {"role": "developer", "content": [{"type": "text", "text": "Be brief."}]},
            {"role": "user", "content": "Hi"},
        ])));
        assert_eq!(req["system"], "You are a.\n\nBe brief.");
        assert_eq!(req["max_tokens"], BUDGET);
        assert_eq!(req["model"], "claude-opus-5-5");
        assert_eq!(
            req["messages"],
            json!([{"role": "user", "content": [{"type": "text", "text": "Hi"}]}])
        );
        let mut named = chat(json!([]));
        named["max_completion_tokens"] = json!(512);
        assert_eq!(body(&named)["max_tokens"], 512);
        named["max_tokens"] = json!(64);
        assert_eq!(body(&named)["max_tokens"], 512, "the newer field wins");
        let configured = translate(&chat(json!([])), 64_000).0;
        assert_eq!(configured["max_tokens"], 64_000);
        assert_eq!(body(&chat(json!([]))).get("system"), None);
    }

    #[test]
    fn sampling_parameters_are_dropped() {
        let mut c = chat(json!([{"role": "user", "content": "Hi"}]));
        for (k, v) in [("temperature", 0.2), ("top_p", 0.9), ("top_k", 5.0)] {
            c[k] = json!(v);
        }
        c["frequency_penalty"] = json!(0.5);
        let req = body(&c);
        let mut keys: Vec<&String> = req.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["max_tokens", "messages", "model"]);
    }

    #[test]
    fn tools_and_tool_choice_map() {
        let mut c = chat(json!([]));
        c["tools"] = json!([
            {"type": "function", "function": {"name": "get", "description": "Gets.",
             "parameters": {"type": "object", "properties": {"id": {"type": "string"}}}}},
            {"type": "function", "function": {"name": "bare"}},
        ]);
        let req = body(&c);
        assert_eq!(
            req["tools"],
            json!([
                {"name": "get", "description": "Gets.",
                 "input_schema": {"type": "object", "properties": {"id": {"type": "string"}}}},
                {"name": "bare", "input_schema": {"type": "object", "properties": {}}},
            ])
        );
        assert_eq!(req.get("tool_choice"), None);
        let named = json!({"type": "function", "function": {"name": "get"}});
        for (choice, kind) in [
            (json!("auto"), "auto"),
            (json!("required"), "auto"),
            (named, "auto"),
            (json!("none"), "none"),
        ] {
            c["tool_choice"] = choice;
            assert_eq!(body(&c)["tool_choice"], json!({"type": kind}));
        }
    }

    #[test]
    fn tool_calls_and_parallel_results_round_trip() {
        let req = body(&chat(json!([
            {"role": "user", "content": "Look both up."},
            {"role": "assistant", "content": "Checking.",
             "tool_calls": [call("t1", "get", r#"{"id":"a"}"#), call("t2", "get", "{bad")]},
            tool_msg("t1", "A"),
            tool_msg("t2", "B"),
            {"role": "assistant", "content": "Both done."},
        ])));
        assert_eq!(
            req["messages"],
            json!([
                {"role": "user", "content": [{"type": "text", "text": "Look both up."}]},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "Checking."},
                    {"type": "tool_use", "id": "t1", "name": "get", "input": {"id": "a"}},
                    {"type": "tool_use", "id": "t2", "name": "get", "input": {}},
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "A"},
                    {"type": "tool_result", "tool_use_id": "t2", "content": "B"},
                ]},
                {"role": "assistant", "content": [{"type": "text", "text": "Both done."}]},
            ])
        );
    }

    #[test]
    fn empty_text_is_not_sent() {
        let req = body(&chat(json!([
            {"role": "user", "content": "Go."},
            {"role": "assistant", "content": null, "tool_calls": [call("t1", "get", "")]},
            tool_msg("t1", "ok"),
        ])));
        let assistant = &req["messages"][1]["content"];
        assert_eq!(
            assistant,
            &json!([{"type": "tool_use", "id": "t1", "name": "get", "input": {}}])
        );
    }

    /// An assistant message as the response translation leaves it, made under `context`.
    fn with_claude_blocks(context: &str) -> Value {
        json!({"role": "assistant", "content": null,
               "tool_calls": [call("t1", "get", "{}")],
               CONTENT_KEY: [
                   {"type": "thinking", "thinking": "", "signature": "sig"},
                   {"type": "tool_use", "id": "t1", "name": "get", "input": {}},
               ],
               CONTEXT_KEY: context})
    }

    #[test]
    fn claude_blocks_go_back_unchanged_in_the_same_context() {
        let system = json!({"role": "system", "content": "You are a."});
        let user = json!({"role": "user", "content": "Go."});
        let (_, context) = translate(&chat(json!([system, user])), BUDGET);
        let next = |sys: &Value| {
            let msgs = json!([
                sys,
                user,
                with_claude_blocks(&context),
                tool_msg("t1", "ok")
            ]);
            body(&chat(msgs))["messages"][1]["content"].clone()
        };
        assert_eq!(next(&system), with_claude_blocks(&context)[CONTENT_KEY]);
        // Under another system prompt the thinking would be refused: it is left out.
        let changed = json!({"role": "system", "content": "You are b."});
        assert_eq!(
            next(&changed),
            json!([{"type": "tool_use", "id": "t1", "name": "get", "input": {}}])
        );
    }

    #[test]
    fn parts_that_are_not_text_leave_a_placeholder() {
        let image = json!({"type": "image_url", "image_url": {"url": "data:..."}});
        let req = body(&chat(json!([
            {"role": "user", "content": [{"type": "text", "text": "Look:"}, image]},
            {"role": "tool", "tool_call_id": "t1", "content": [{"type": "input_audio"}]},
        ])));
        let user = &req["messages"][0]["content"];
        assert_eq!(user[1]["text"], "[image omitted: Arena sends text only]");
        assert_eq!(user[2]["content"], "[audio omitted: Arena sends text only]");
    }

    #[test]
    fn error_results_are_marked() {
        let req = body(&chat(json!([
            {"role": "user", "content": "Go."},
            {"role": "assistant", "content": null,
             "tool_calls": [call("t1", "get", "{}"), call("t2", "get", "{}")]},
            tool_msg("t1", r#"{"error":"no such order"}"#),
            tool_msg("t2", r#"{"status":"ok"}"#),
        ])));
        let results = &req["messages"][2]["content"];
        assert_eq!(results[0]["is_error"], true);
        assert_eq!(results[1].get("is_error"), None);
    }

    #[test]
    fn the_context_ignores_the_order_of_tools() {
        let tool = |name: &str| json!({"type": "function", "function": {"name": name}});
        let mut c = chat(json!([{"role": "user", "content": "Hi"}]));
        c["tools"] = json!([tool("a"), tool("b")]);
        let (_, ab) = translate(&c, BUDGET);
        c["tools"] = json!([tool("b"), tool("a")]);
        assert_eq!(translate(&c, BUDGET).1, ab);
        c["tools"] = json!([tool("a")]);
        assert_ne!(translate(&c, BUDGET).1, ab);
    }
}
