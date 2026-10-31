//! Drives `serve_on` with in-memory pipes and the counter example part.
#[allow(dead_code)]
#[path = "../examples/counter.rs"]
mod counter;

use serde_json::{json, Value};
use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

struct Client {
    w: DuplexStream,
    r: tokio::io::Lines<BufReader<DuplexStream>>,
    next: u64,
}

impl Client {
    fn new(flavour_part: counter::Counter) -> Client {
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
async fn save_then_start_from_save_restores() {
    let mut c = client();
    let dir = std::env::temp_dir().join(format!("arena-part-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    start(&mut c, "i1").await;
    add(&mut c, "i1", 5).await;
    let r = c
        .call(
            "save",
            json!({"instance": "i1", "save": "s1", "dir": dir.to_str().unwrap()}),
        )
        .await;
    assert_eq!(r["result"]["exact"], true, "{r}");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn requests_answered_concurrently() {
    let mut c = client();
    let a = c.send("describe", json!({})).await;
    let b = c.send("usage", json!({})).await;
    let mut seen = vec![
        c.recv().await["id"].as_u64().unwrap(),
        c.recv().await["id"].as_u64().unwrap(),
    ];
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
