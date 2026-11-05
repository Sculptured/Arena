//! Live test of the twenty-crm integration: a database server and Twenty servers of the test's own
//! (compose project `arena_twenty_t<pid>`, image `twentycrm/twenty:v2.51.0`, never pulled), and the
//! conformance check against them. Everything it makes is removed when it ends, even when it fails.
//! Run: `cargo test -p arena-twenty-crm -- --ignored live_ --nocapture`.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use arena_part::proto::*;
use arena_part::Part;
use arena_twenty_crm::{Settings, TwentyCrm};
use serde_json::{json, Value};

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The test's server and files; all removed on drop.
struct Env {
    settings: Settings,
    saves: PathBuf,
}

impl Env {
    fn new() -> Env {
        let pid = std::process::id();
        let settings = Settings {
            project: format!("arena_twenty_t{pid}"),
            db_port: free_port(),
            ..Settings::default()
        };
        let saves = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("twenty-{pid}"));
        std::fs::create_dir_all(&saves).unwrap();
        Env { settings, saves }
    }

    fn part(&self) -> TwentyCrm {
        TwentyCrm::new(self.settings.clone(), self.saves.join("twenty-crm"))
    }

    fn config(&self) -> Value {
        json!({"project": self.settings.project, "db_port": self.settings.db_port})
    }

    /// Every container of the test's prefix, then its networks and its volume.
    fn remove_all(&self) {
        let name = self.settings.docker_name();
        for c in listed(&["ps", "-aq", "--filter", &format!("name=^{name}")]) {
            docker(&["rm", "-f", "-v", &c]);
        }
        let project = format!("name={}", self.settings.project);
        for n in listed(&["network", "ls", "-q", "--filter", &project]) {
            docker(&["network", "rm", &n]);
        }
        for n in listed(&["network", "ls", "-q", "--filter", &format!("name={name}")]) {
            docker(&["network", "rm", &n]);
        }
        docker(&["volume", "rm", "-f", &format!("{name}-db")]);
    }
}

fn docker(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn listed(args: &[&str]) -> Vec<String> {
    let text = docker(args);
    text.split_whitespace().map(str::to_string).collect()
}

impl Drop for Env {
    fn drop(&mut self) {
        self.remove_all();
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn start(e: &Env, instance: &str, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config: e.config(),
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn tool(p: &TwentyCrm, instance: &str, name: &str, args: Value) -> ToolOutput {
    let call = CallToolParams {
        instance: instance.into(),
        name: name.into(),
        args,
        clock_ms: 0,
    };
    p.call_tool(call).await.unwrap()
}

fn result(out: ToolOutput) -> Value {
    assert_eq!(out.error, None);
    out.result.unwrap()
}

fn inst(instance: &str) -> Target {
    Target::Instance {
        instance: instance.into(),
    }
}

fn saved(save: &str) -> Target {
    Target::Save { save: save.into() }
}

fn id(instance: &str) -> IdParams {
    IdParams {
        instance: instance.into(),
    }
}

/// `POST /rest/<path>` as a benchmark's seed writes it over `connect`; the new record's id.
async fn post(info: &Value, path: &str, made: &str, body: Value) -> String {
    let at = format!("{}/rest/{path}", info["url"].as_str().unwrap());
    let key = info["api_key"].as_str().unwrap();
    let client = reqwest::Client::new();
    let r = client.post(at).bearer_auth(key).json(&body).send().await;
    let r = r.unwrap();
    assert!(
        r.status().is_success(),
        "{path}: {}",
        r.text().await.unwrap()
    );
    let answer: Value = r.json().await.unwrap();
    let new = &answer["data"][format!("create{made}")]["id"];
    new.as_str().unwrap().to_string()
}

/// An account, a contact and an opportunity.
async fn seed_account(p: &TwentyCrm, instance: &str) -> (String, String) {
    let info = p.connect(id(instance)).await.unwrap();
    let company = json!({"name": "Northfield Motors"});
    let account = post(&info, "companies", "Company", company).await;
    let person = json!({"name": {"firstName": "Avery", "lastName": "Cole"},
                        "emails": {"primaryEmail": "avery@northfield.example"},
                        "jobTitle": "Buyer", "companyId": account});
    post(&info, "people", "Person", person).await;
    let deal = json!({"name": "A-1 RC-1041", "stage": "PROPOSAL", "closeDate": "2026-11-05",
                      "amount": {"amountMicros": 57600000000u64, "currencyCode": "USD"},
                      "companyId": account});
    let deal = post(&info, "opportunities", "Opportunity", deal).await;
    (account, deal)
}

async fn all_docs(p: &TwentyCrm, instance: &str) -> serde_json::Map<String, Value> {
    let params = InspectParams {
        target: inst(instance),
        keys: vec![],
    };
    p.inspect(params).await.unwrap().docs
}

async fn check_base_is_empty(p: &TwentyCrm) {
    let docs = all_docs(p, "live_a").await;
    assert!(docs.is_empty(), "demo records left: {:?}", docs.keys());
}

async fn check_finds(p: &TwentyCrm, account: &str, deal: &str) {
    let a = result(
        tool(
            p,
            "live_a",
            "crm_find_account",
            json!({"name": "northfield"}),
        )
        .await,
    );
    let a = &a["accounts"][0];
    assert_eq!(a["id"], account);
    assert_eq!(a["contacts"][0]["name"], "Avery Cole");
    assert_eq!(a["opportunities"][0]["close_date"], "2026-11-05");
    let named = json!({"name": "Avery Cole"});
    let named = result(tool(p, "live_a", "crm_find_contact", named).await);
    assert_eq!(named["contacts"][0]["email"], "avery@northfield.example");
    let mailed = json!({"email": "NORTHFIELD", "account_id": account});
    let mailed = result(tool(p, "live_a", "crm_find_contact", mailed).await);
    assert_eq!(mailed["contacts"].as_array().unwrap().len(), 1);
    let none = json!({"name": "Nobody"});
    let none = result(tool(p, "live_a", "crm_find_contact", none).await);
    assert_eq!(none["contacts"], json!([]));
    let deals = json!({"account_id": account});
    let deals = result(tool(p, "live_a", "crm_find_opportunities", deals).await);
    assert_eq!(deals["opportunities"][0]["id"], deal);
}

async fn check_writes(p: &TwentyCrm, account: &str, deal: &str) {
    let moved = json!({"id": deal, "close_date": "2026-11-09", "stage": "MEETING"});
    let moved = result(tool(p, "live_a", "crm_update_opportunity", moved).await);
    assert_eq!(moved["close_date"], "2026-11-09");
    assert_eq!(moved["stage"], "MEETING");
    let refused = json!({"account_id": "00000000-0000-4000-8000-000000000000",
                         "title": "t", "body": "b"});
    let refused = tool(p, "live_a", "crm_log_note", refused).await;
    assert!(refused.error.is_some());
    let note = json!({"account_id": account, "title": "PR-A1 down", "body": "Press **down**."});
    let out = tool(p, "live_a", "crm_log_note", note).await;
    assert_eq!(out.writes.len(), 2, "{out:?}");
    let task = json!({"title": "Call Avery", "due": "2026-11-06", "account_id": account});
    let out = tool(p, "live_a", "crm_create_task", task).await;
    assert_eq!(out.writes.len(), 2, "{out:?}");
}

/// The one document of `kind`.
fn only<'a>(docs: &'a serde_json::Map<String, Value>, kind: &str) -> &'a Value {
    let prefix = format!("{kind}/");
    let mut of_kind = docs.iter().filter(|(k, _)| k.starts_with(&prefix));
    of_kind.next().unwrap().1
}

async fn check_inspect(p: &TwentyCrm, deal: &str) {
    let docs = all_docs(p, "live_a").await;
    let note = only(&docs, "note");
    assert_eq!(note["accounts"], json!(["Northfield Motors"]));
    assert!(note["bodyV2Markdown"].as_str().unwrap().contains("Press"));
    assert_eq!(
        only(&docs, "task")["accounts"],
        json!(["Northfield Motors"])
    );
    let key = format!("opportunity/{deal}");
    assert_eq!(docs[&key]["account"], "Northfield Motors");
    let date = docs[&key]["closeDate"].as_str().unwrap();
    assert!(date.starts_with("2026-11-09"), "{date}");
}

async fn save(p: &TwentyCrm, instance: &str, save: &str) -> Saved {
    let params = SaveParams {
        instance: instance.into(),
        save: save.into(),
        dir: String::new(),
    };
    p.save(params).await.unwrap()
}

async fn differing(p: &TwentyCrm, a: Target, b: Target) -> Vec<String> {
    p.diff(DiffParams { a, b }).await.unwrap().differing
}

/// A save, a change, a restore: the restored instance is the save, with a server that works.
async fn check_restore(p: &TwentyCrm, e: &Env, deal: &str) {
    let t = Instant::now();
    let done = save(p, "live_a", "live_s1").await;
    eprintln!("save: {:?}, {} bytes", t.elapsed(), done.bytes);
    assert!(done.exact && done.bytes > 0);
    // the server survives having its connections closed for the copy
    let later = json!({"id": deal, "close_date": "2026-12-25"});
    result(tool(p, "live_a", "crm_update_opportunity", later).await);
    let diff = differing(p, inst("live_a"), saved("live_s1")).await;
    assert_eq!(diff, [format!("opportunity/{deal}")]);
    let t = Instant::now();
    p.start(start(e, "live_b", Some("live_s1"))).await.unwrap();
    eprintln!("restore: {:?}", t.elapsed());
    assert!(differing(p, inst("live_b"), saved("live_s1"))
        .await
        .is_empty());
    let deals = result(tool(p, "live_b", "crm_find_opportunities", json!({})).await);
    assert_eq!(deals["opportunities"][0]["close_date"], "2026-11-09");
    p.stop(id("live_b")).await.unwrap();
}

/// A new program finds what the last one left running.
async fn check_restarted_program(e: &Env, deal: &str) {
    let p = e.part();
    let deals = result(tool(&p, "live_a", "crm_find_opportunities", json!({})).await);
    assert_eq!(deals["opportunities"][0]["id"], deal);
    save(&p, "live_a", "live_s2").await;
    let s2 = SaveId {
        save: "live_s2".into(),
    };
    p.drop_save(s2.clone()).await.unwrap();
    assert_eq!(p.drop_save(s2).await.unwrap_err().code, -32002);
    let gone = p.gc(GcParams { keep: vec![] }).await.unwrap();
    assert_eq!(
        gone.dropped, 1,
        "the unkept save goes, the live instance stays"
    );
    p.stop(id("live_a")).await.unwrap();
    assert_eq!(p.usage().await.unwrap().bytes, 0);
}

fn cargo(dir: PathBuf, args: &[&str]) {
    let status = Command::new("cargo").args(args).current_dir(dir).status();
    assert!(status.unwrap().success(), "cargo {args:?}");
}

/// `arena-part-check` on a program of its own, against the test's server.
fn conformance(e: &Env) {
    let a = arena();
    let program = Path::new(env!("CARGO_BIN_EXE_arena-twenty-crm"));
    let bin = program.parent().unwrap();
    cargo(
        a.join("core"),
        &["build", "-p", "parts", "--bin", "arena-part-check"],
    );
    let config = e.saves.join("check.json");
    std::fs::write(&config, e.config().to_string()).unwrap();
    let t = Instant::now();
    let out = Command::new(bin.join("arena-part-check"))
        .arg(a.join("integrations/twenty-crm/check"))
        .arg("--config")
        .arg(&config)
        .env("ARENA_BIN_DIR", bin)
        .env("ARENA_SAVES", &e.saves)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    eprintln!("conformance ({:?}):\n{stdout}", t.elapsed());
    assert!(
        out.status.success() && stdout.contains("\"passed\": true"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
#[ignore = "needs Docker, twentycrm/twenty:v2.51.0, postgres:17-alpine and redis:6.2-alpine"]
async fn live_twenty_roundtrip() {
    let e = Env::new();
    let p = e.part();
    let t = Instant::now();
    p.start(start(&e, "live_a", None)).await.unwrap();
    eprintln!("first start: {:?}", t.elapsed());
    check_base_is_empty(&p).await;
    let (account, deal) = seed_account(&p, "live_a").await;
    check_finds(&p, &account, &deal).await;
    check_writes(&p, &account, &deal).await;
    check_inspect(&p, &deal).await;
    check_restore(&p, &e, &deal).await;
    check_restarted_program(&e, &deal).await;
    conformance(&e);
}
