//! The `list_files`, `read_file` and `write_file` tools.
use std::fs;
use std::path::Path;

use arena_part::helpers::folder;
use arena_part::proto::{Effect, ToolOutput, ToolSchema};
use serde_json::{json, Value};

use crate::paths::resolve;

/// The key a file has in `reads`, `writes` and `inspect`.
pub fn file_key(rel: &str) -> String {
    format!("file/{rel}")
}

fn schema(name: &str, description: &str, props: Value, effect: Effect) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": props}),
        effect,
    }
}

pub fn schemas() -> Vec<ToolSchema> {
    let path = json!({"type": "string", "description": "path inside the folder"});
    vec![
        schema(
            "list_files",
            "List every file in the folder with its size.",
            json!({}),
            Effect::Read,
        ),
        schema(
            "read_file",
            "Read a text file.",
            json!({"path": path}),
            Effect::Read,
        ),
        schema(
            "write_file",
            "Write a text file, replacing it if it exists.",
            json!({"path": path, "content": {"type": "string"}}),
            Effect::Write,
        ),
    ]
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn io_err(e: std::io::Error) -> String {
    e.to_string()
}

fn ok(result: Value, reads: Vec<String>, writes: Vec<String>) -> ToolOutput {
    ToolOutput {
        result: Some(result),
        error: None,
        reads,
        writes,
        ..ToolOutput::default()
    }
}

fn list_files(root: &Path) -> Result<ToolOutput, String> {
    let files: Vec<Value> = folder::list(root)
        .map_err(io_err)?
        .into_iter()
        .map(|e| {
            let bytes = fs::metadata(&e.abs).map_or(0, |m| m.len());
            json!({"path": e.path, "bytes": bytes})
        })
        .collect();
    Ok(ok(json!(files), vec![file_key("*")], vec![]))
}

fn read_file(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let at = resolve(root, str_arg(args, "path")?, false)?;
    let bytes = fs::read(&at.abs).map_err(io_err)?;
    let content = String::from_utf8(bytes).map_err(|_| format!("{} is not text", at.rel))?;
    Ok(ok(
        json!({"content": content}),
        vec![file_key(&at.rel)],
        vec![],
    ))
}

fn write_file(root: &Path, args: &Value) -> Result<ToolOutput, String> {
    let at = resolve(root, str_arg(args, "path")?, false)?;
    let content = str_arg(args, "content")?;
    if let Some(dir) = at.abs.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    fs::write(&at.abs, content).map_err(io_err)?;
    Ok(ok(
        json!({"bytes": content.len()}),
        vec![],
        vec![file_key(&at.rel)],
    ))
}

/// Runs the tool `name` on the instance folder `root`; a failure is the output's `error`.
pub fn run(root: &Path, name: &str, args: &Value) -> ToolOutput {
    let out = match name {
        "list_files" => list_files(root),
        "read_file" => read_file(root, args),
        "write_file" => write_file(root, args),
        _ => Err(format!("unknown tool {name}")),
    };
    out.unwrap_or_else(|error| ToolOutput {
        error: Some(error),
        ..ToolOutput::default()
    })
}
