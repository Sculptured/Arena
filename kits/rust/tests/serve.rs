//! Drives `serve_on` with in-memory pipes and the counter example part.
#[allow(dead_code)]
#[path = "../examples/counter.rs"]
mod counter;

use arena_part::proto::*;
use arena_part::PartError;
use serde_json::{json, Value};
use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

struct Client {
    w: DuplexStream,
    r: tokio::io::Lines<BufReader<DuplexStream>>,
    next: u64,
}

impl Client {
    fn new<P: arena_part::Part>(flavour_part: P) -> Client {
        let (cw, sr) = duplex(1 << 16);
        let (sw, cr) = duplex(1 << 16);
        tokio::spawn(async move {
            let _ = arena_part::serve_on(flavour_part, BufReader::new(sr), sw).await;
        });
        Client {
            w: cw,
            r: BufReader::new(cr).lines(),
            next: 1,
        }
    }
    async fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.w
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        id
    }
    async fn recv(&mut self) -> Value {
        serde_json::from_str(&self.r.next_line().await.unwrap().unwrap()).unwrap()
    }
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.send(method, params).await;
        self.recv().await
    }
}

fn client() -> Client {
    Client::new(counter::Counter::default())
}

fn flavour_client(f: &str) -> Client {
    Client::new(counter::Counter::new(counter::Flavour::parse(f).unwrap()))
}

/// Sets ARENA_SAVES once for this test process; returns the folder of `<save>` for counter.
fn save_dir(save: &str) -> String {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let root = ROOT.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("arena-part-test-{}", std::process::id()));
        std::env::set_var("ARENA_SAVES", &d);
        d
    });
    root.join("counter")
        .join(save)
        .to_str()
        .unwrap()
        .to_string()
}

/// A part with deliberately odd handlers: `panic`, `wait` (blocks until `poke`).
#[derive(Default)]
struct Odd(std::sync::Arc<tokio::sync::Notify>);

#[async_trait::async_trait]
impl arena_part::Part for Odd {
    async fn describe(&self) -> Result<Describe, PartError> {
        Err(PartError::internal("n/a"))
    }
    async fn start(&self, _: StartParams) -> Result<(), PartError> {
        Ok(())
    }
    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        match p.name.as_str() {
            "panic" => panic!("boom"),
            "wait" => self.0.notified().await,
            _ => self.0.notify_one(),
        }
        Ok(ToolOutput::default())
    }
    async fn save(&self, _: SaveParams) -> Result<Saved, PartError> {
        Err(PartError::internal("n/a"))
    }
    async fn inspect(&self, _: InspectParams) -> Result<Docs, PartError> {
        Err(PartError::internal("n/a"))
    }
    async fn diff(&self, _: DiffParams) -> Result<Differing, PartError> {
        Err(PartError::internal("n/a"))
    }
    async fn drop_save(&self, _: SaveId) -> Result<(), PartError> {
        Ok(())
    }
    async fn stop(&self, _: IdParams) -> Result<(), PartError> {
        Ok(())
    }
    async fn usage(&self) -> Result<Usage, PartError> {
        Ok(Usage::default())
    }
}

async fn start(c: &mut Client, instance: &str) {
    let r = c
        .call(
            "start",
            json!({"instance": instance, "config": {}, "seed": 1, "clock_ms": 0}),
        )
        .await;
    assert!(r.get("error").is_none(), "{r}");
}

async fn get(c: &mut Client, instance: &str) -> i64 {
    let r = c
        .call(
            "call_tool",
            json!({"instance": instance, "name": "get", "args": {}, "clock_ms": 0}),
        )
        .await;
    r["result"]["result"]["value"]
        .as_i64()
        .unwrap_or_else(|| panic!("{r}"))
}

async fn add(c: &mut Client, instance: &str, n: i64) {
    let r = c
        .call(
            "call_tool",
            json!({"instance": instance, "name": "add", "args": {"n": n}, "clock_ms": 0}),
        )
        .await;
    assert!(r.get("error").is_none(), "{r}");
}

#[tokio::test]
async fn describe_reports_protocol_1() {
    let mut c = client();
    let r = c.call("describe", json!({})).await;
    assert_eq!(r["result"]["protocol"], 1);
    assert!(r["result"]["tools"][0].get("inputSchema").is_some());
}

#[tokio::test]
async fn unknown_method_is_32601() {
    let mut c = client();
    let r = c.call("nope", json!({})).await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn bad_instance_id_is_32602() {
    let mut c = client();
    let r = c
        .call(
            "start",
            json!({"instance": "Bad Id", "config": {}, "seed": 1, "clock_ms": 0}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn gc_keep_takes_ids_and_prefixes() {
    let mut c = client();
    let r = c.call("gc", json!({"keep": ["s_1", "base_*", "*"]})).await;
    assert_eq!(r["result"]["dropped"], 0, "{r}");
    let r = c.call("gc", json!({"keep": ["Bad*"]})).await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

#[tokio::test]
async fn save_then_start_from_save_restores() {
    let mut c = client();
    start(&mut c, "i1").await;
    add(&mut c, "i1", 5).await;
    let r = c
        .call(
            "save",
            json!({"instance": "i1", "save": "s1", "dir": save_dir("s1")}),
        )
        .await;
    assert_eq!(r["result"]["exact"], true, "{r}");
    assert_eq!(r["result"]["bytes"], 20, "{r}");
    add(&mut c, "i1", 2).await;
    let r = c
        .call(
            "start",
            json!({"instance": "i2", "config": {}, "seed": 1, "clock_ms": 0,
                   "from": {"save": "s1", "mode": "exact"}}),
        )
        .await;
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(get(&mut c, "i2").await, 5);
    assert_eq!(get(&mut c, "i1").await, 7);
}

#[tokio::test]
async fn fresh_process_starts_from_other_processs_save() {
    let mut a = client();
    start(&mut a, "i1").await;
    add(&mut a, "i1", 9).await;
    a.call(
        "save",
        json!({"instance": "i1", "save": "s_x", "dir": save_dir("s_x")}),
    )
    .await;
    let mut b = client();
    let r = b
        .call(
            "start",
            json!({"instance": "i1", "config": {}, "seed": 1, "clock_ms": 0,
                   "from": {"save": "s_x", "mode": "exact"}}),
        )
        .await;
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(get(&mut b, "i1").await, 9);
    let r = b
        .call("inspect", json!({"target": {"save": "s_x"}, "keys": []}))
        .await;
    assert_eq!(r["result"]["docs"]["counter/value"], 9);
}

#[tokio::test]
async fn unknown_save_is_32002() {
    let mut c = client();
    save_dir("x");
    let r = c
        .call(
            "start",
            json!({"instance": "i1", "config": {}, "seed": 1, "clock_ms": 0,
                   "from": {"save": "nope", "mode": "exact"}}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let r = c
        .call("inspect", json!({"target": {"save": "nope"}, "keys": []}))
        .await;
    assert_eq!(r["error"]["code"], -32002);
}

#[tokio::test]
async fn flavours_describe_their_restore() {
    for (f, exact, approx) in [
        ("exact", "save", false),
        ("replay_only", "replay", false),
        ("approx_and_replay", "replay", true),
        ("broken_restore", "save", false),
        ("nondeterministic", "replay", false),
    ] {
        let r = flavour_client(f).call("describe", json!({})).await;
        assert_eq!(
            r["result"]["restore"],
            json!({"exact": exact, "approximate": approx}),
            "{f}"
        );
    }
}

#[tokio::test]
async fn replay_only_save_is_32001() {
    let mut c = flavour_client("replay_only");
    start(&mut c, "i1").await;
    let r = c
        .call(
            "save",
            json!({"instance": "i1", "save": "s_r", "dir": save_dir("s_r")}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
}

#[tokio::test]
async fn approx_save_reports_inexact() {
    let mut c = flavour_client("approx_and_replay");
    start(&mut c, "i1").await;
    let r = c
        .call(
            "save",
            json!({"instance": "i1", "save": "s_a", "dir": save_dir("s_a")}),
        )
        .await;
    assert_eq!(r["result"]["exact"], false, "{r}");
}

#[tokio::test]
async fn broken_restore_restores_zero() {
    let mut c = flavour_client("broken_restore");
    start(&mut c, "i1").await;
    add(&mut c, "i1", 5).await;
    c.call(
        "save",
        json!({"instance": "i1", "save": "s_b", "dir": save_dir("s_b")}),
    )
    .await;
    c.call(
        "start",
        json!({"instance": "i2", "config": {}, "seed": 1, "clock_ms": 0,
               "from": {"save": "s_b", "mode": "exact"}}),
    )
    .await;
    assert_eq!(get(&mut c, "i2").await, 0);
}

#[tokio::test]
async fn nondeterministic_add_varies() {
    let mut c = flavour_client("nondeterministic");
    start(&mut c, "i1").await;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        add(&mut c, "i1", 0).await;
        seen.insert(get(&mut c, "i1").await);
    }
    assert!(seen.len() > 1);
}

#[tokio::test]
async fn panicking_handler_still_answers() {
    let mut c = Client::new(Odd::default());
    let r = c
        .call(
            "call_tool",
            json!({"instance": "i1", "name": "panic", "args": {}, "clock_ms": 0}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32603, "{r}");
    assert!(
        r["error"]["message"].as_str().unwrap().contains("boom"),
        "{r}"
    );
    // The server keeps serving.
    let r = c.call("usage", json!({})).await;
    assert!(r.get("result").is_some(), "{r}");
}

#[tokio::test]
async fn requests_answered_concurrently() {
    // `wait` blocks until `poke` arrives; both can only finish if they run at the same time.
    let mut c = Client::new(Odd::default());
    let call = |name: &str| json!({"instance": "i1", "name": name, "args": {}, "clock_ms": 0});
    let a = c.send("call_tool", call("wait")).await;
    let b = c.send("call_tool", call("poke")).await;
    let mut seen = Vec::new();
    for _ in 0..2 {
        let r = tokio::time::timeout(std::time::Duration::from_secs(5), c.recv())
            .await
            .unwrap();
        seen.push(r["id"].as_u64().unwrap());
    }
    seen.sort();
    assert_eq!(seen, vec![a, b]);
}

#[tokio::test]
async fn advance_is_split_independent() {
    let mut one = client();
    let mut many = client();
    start(&mut one, "i1").await;
    start(&mut many, "i1").await;
    one.call("advance", json!({"instance": "i1", "clock_ms": 2500}))
        .await;
    for t in [500, 1000, 1500, 2000, 2500] {
        many.call("advance", json!({"instance": "i1", "clock_ms": t}))
            .await;
    }
    assert_eq!(get(&mut one, "i1").await, 2);
    assert_eq!(get(&mut many, "i1").await, 2);
}
