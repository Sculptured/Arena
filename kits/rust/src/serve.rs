//! Line protocol loop, method dispatch and id validation.
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::error::PartError;
use crate::proto::*;

type R<T> = Result<T, PartError>;

/// An integration: one program that holds many instances and saves of one kind of system.
#[async_trait]
pub trait Part: Send + Sync + 'static {
    async fn describe(&self) -> R<Describe>;
    async fn start(&self, p: StartParams) -> R<()>;
    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput>;
    async fn advance(&self, _p: AdvanceParams) -> R<Advanced> {
        Ok(Advanced::default())
    }
    async fn save(&self, p: SaveParams) -> R<Saved>;
    async fn inspect(&self, p: InspectParams) -> R<Docs>;
    async fn diff(&self, p: DiffParams) -> R<Differing>;
    async fn drop_save(&self, p: SaveId) -> R<()>;
    async fn stop(&self, p: IdParams) -> R<()>;
    async fn usage(&self) -> R<Usage>;
    async fn gc(&self, _p: GcParams) -> R<GcResult> {
        Ok(GcResult::default())
    }
    async fn connect(&self, _p: IdParams) -> R<Value> {
        Err(PartError::method_not_found("connect"))
    }
}

/// A benchmark: tasks, tools and a loader for the parts of its world.
#[async_trait]
pub trait Benchmark: Send + Sync + 'static {
    async fn describe(&self) -> R<BenchDescribe>;
    async fn seed(&self, p: SeedParams) -> R<()>;
    async fn call_tool(&self, p: BenchCallTool) -> R<BenchToolOutput>;
    async fn gold_actions(&self, p: GoldParams) -> R<GoldActions>;
}

/// Serve a part on stdin and stdout until stdin closes.
pub async fn serve<P: Part>(p: P) -> anyhow::Result<()> {
    serve_on(p, BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

/// Serve a benchmark on stdin and stdout until stdin closes.
pub async fn serve_benchmark<B: Benchmark>(b: B) -> anyhow::Result<()> {
    serve_benchmark_on(b, BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

/// Like [`serve`], over any reader and writer (for tests).
pub async fn serve_on<P, Rd, W>(p: P, r: Rd, w: W) -> anyhow::Result<()>
where
    P: Part,
    Rd: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let p = Arc::new(p);
    run_loop(r, w, move |m, params| {
        let p = p.clone();
        async move { part_dispatch(&*p, &m, params).await }
    })
    .await
}

/// Like [`serve_benchmark`], over any reader and writer (for tests).
pub async fn serve_benchmark_on<B, Rd, W>(b: B, r: Rd, w: W) -> anyhow::Result<()>
where
    B: Benchmark,
    Rd: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let b = Arc::new(b);
    run_loop(r, w, move |m, params| {
        let b = b.clone();
        async move { bench_dispatch(&*b, &m, params).await }
    })
    .await
}

/// Read request lines; run each on its own task; write each response as one whole line.
async fn run_loop<Rd, W, F, Fut>(mut r: Rd, w: W, handler: F) -> anyhow::Result<()>
where
    Rd: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = R<Value>> + Send + 'static,
{
    let w = Arc::new(Mutex::new(w));
    let handler = Arc::new(handler);
    let mut tasks = JoinSet::new();
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                let err = PartError {
                    code: -32700,
                    message: format!("parse error: {e}"),
                };
                write_line(&w, response(Value::Null, Err(err))).await?;
                continue;
            }
        };
        let (w, handler) = (w.clone(), handler.clone());
        tasks.spawn(async move {
            let id = req.get("id").cloned();
            let method = req
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let out = if method.is_empty() {
                Err(PartError {
                    code: -32600,
                    message: "invalid request".into(),
                })
            } else if !ids_ok(&params) {
                Err(PartError::invalid(
                    "instance and save ids must match [a-z0-9_]{1,63}",
                ))
            } else {
                // Run on its own task so a panic is caught and answered.
                match tokio::spawn(handler(method, params)).await {
                    Ok(r) => r,
                    Err(e) => Err(PartError::internal(format!(
                        "internal error: {}",
                        panic_message(e)
                    ))),
                }
            };
            // Notifications (no id) get no response.
            if let Some(id) = id {
                if let Err(e) = write_line(&w, response(id, out)).await {
                    eprintln!("arena-part: write failed: {e}");
                }
            }
        });
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

fn panic_message(e: tokio::task::JoinError) -> String {
    if !e.is_panic() {
        return e.to_string();
    }
    let p = e.into_panic();
    match p.downcast::<String>() {
        Ok(s) => *s,
        Err(p) => p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "panic".into()),
    }
}

async fn write_line<W: AsyncWrite + Unpin>(w: &Mutex<W>, v: Value) -> anyhow::Result<()> {
    let mut s = serde_json::to_string(&v)?;
    s.push('\n');
    let mut w = w.lock().await;
    w.write_all(s.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

fn response(id: Value, out: R<Value>) -> Value {
    match out {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(e) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": e.code, "message": e.message}})
        }
    }
}

/// Every `instance`, `save`, `keep` entry (also inside `target`, `a`, `b`, `from`) must be a valid id.
fn ids_ok(params: &Value) -> bool {
    let Some(o) = params.as_object() else {
        return true;
    };
    for (k, v) in o {
        let ok = match k.as_str() {
            "instance" | "save" => v.as_str().is_some_and(valid_id),
            "keep" => v
                .as_array()
                .is_none_or(|a| a.iter().all(|s| s.as_str().is_some_and(valid_id))),
            "target" | "a" | "b" | "from" => ids_ok(v),
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

fn parse<T: DeserializeOwned>(params: Value) -> R<T> {
    serde_json::from_value(params).map_err(|e| PartError::invalid(format!("invalid params: {e}")))
}

fn out<T: Serialize>(v: R<T>) -> R<Value> {
    let v = v?;
    serde_json::to_value(v).map_err(|e| PartError::internal(e.to_string()))
}

async fn part_dispatch<P: Part>(p: &P, method: &str, params: Value) -> R<Value> {
    match method {
        "describe" => out(p.describe().await),
        "start" => out(p.start(parse(params)?).await.map(|_| json!({}))),
        "call_tool" => out(p.call_tool(parse(params)?).await),
        "advance" => out(p.advance(parse(params)?).await),
        "save" => out(p.save(parse(params)?).await),
        "inspect" => out(p.inspect(parse(params)?).await),
        "diff" => out(p.diff(parse(params)?).await),
        "drop_save" => out(p.drop_save(parse(params)?).await.map(|_| json!({}))),
        "stop" => out(p.stop(parse(params)?).await.map(|_| json!({}))),
        "usage" => out(p.usage().await),
        "gc" => out(p.gc(parse(params)?).await),
        "connect" => out(p.connect(parse(params)?).await),
        m => Err(PartError::method_not_found(m)),
    }
}

async fn bench_dispatch<B: Benchmark>(b: &B, method: &str, params: Value) -> R<Value> {
    match method {
        "describe" => out(b.describe().await),
        "seed" => out(b.seed(parse(params)?).await.map(|_| json!({}))),
        "call_tool" => out(b.call_tool(parse(params)?).await),
        "gold_actions" => out(b.gold_actions(parse(params)?).await),
        m => Err(PartError::method_not_found(m)),
    }
}
