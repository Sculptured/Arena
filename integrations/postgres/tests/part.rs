use arena_part::helpers::pg::Templates;
use arena_part::proto::*;
use arena_part::Part;
use arena_postgres::Postgres;
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

fn url() -> String {
    std::env::var("ARENA_PG_URL")
        .unwrap_or_else(|_| "postgres://arena:arena@127.0.0.1:5544/postgres".into())
}

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A test-unique namespace: databases are `pi_<ns><id>` and `ps_<ns><id>`.
fn ns() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "t5_{}_{}_",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// Runs `body`, then drops every database the test could have made, even when `body` panicked.
async fn with_cleanup<F, Fut>(ns: &str, extra: &[String], body: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let res = tokio::spawn(body()).await;
    let t = Templates::new(&url());
    let mut names = extra.to_vec();
    for p in ["pi_", "ps_"] {
        for (n, _) in t.list(&format!("{p}{ns}")).await.unwrap() {
            names.push(n);
        }
    }
    for n in names {
        t.drop(&n).await.unwrap();
    }
    res.unwrap();
}

fn start(instance: &str, config: Value, from: Option<&str>) -> StartParams {
    StartParams {
        instance: instance.into(),
        config,
        seed: 1,
        clock_ms: 0,
        from: from.map(|s| FromSave {
            save: s.into(),
            mode: Mode::Exact,
        }),
    }
}

async fn exec(p: &Postgres, inst: &str, sql: &str) -> ToolOutput {
    p.call_tool(CallToolParams {
        instance: inst.into(),
        name: "sql_execute".into(),
        args: json!({"sql": sql}),
        clock_ms: 0,
    })
    .await
    .unwrap()
}

async fn query(p: &Postgres, inst: &str, sql: &str) -> ToolOutput {
    p.call_tool(CallToolParams {
        instance: inst.into(),
        name: "sql_query".into(),
        args: json!({"sql": sql}),
        clock_ms: 0,
    })
    .await
    .unwrap()
}

fn cfg() -> Value {
    json!({"init_sql": "CREATE TABLE t(id int primary key, v text)"})
}

async fn save(p: &Postgres, inst: &str, save: &str) -> Saved {
    p.save(SaveParams {
        instance: inst.into(),
        save: save.into(),
        dir: "/nonexistent".into(),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn save_restore_roundtrip() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        p.start(start("a", cfg(), None)).await.unwrap();
        let w = exec(&p, "a", "INSERT INTO t(id, v) VALUES (1, 'x')").await;
        assert_eq!(w.error, None);
        assert_eq!(w.result, Some(json!({"rows_affected": 1})));
        assert_eq!(w.writes, vec!["t/*".to_string()]);
        let s = save(&p, "a", "s1").await;
        assert!(s.exact && s.bytes > 0);
        exec(&p, "a", "UPDATE t SET v = 'changed'").await;
        p.start(start("b", cfg(), Some("s1"))).await.unwrap();
        let r = query(&p, "b", "SELECT id, v FROM t").await;
        assert_eq!(r.result, Some(json!([{"id": 1, "v": "x"}])));
        assert_eq!(r.reads, vec!["t/*".to_string()]);
        let c = p
            .connect(IdParams {
                instance: "b".into(),
            })
            .await
            .unwrap();
        assert!(c["url"].as_str().unwrap().ends_with(&format!("/pi_{n}b")));
        let u = p.usage().await.unwrap();
        assert!(u.bytes > 0);
        // an unknown save
        let e = p.start(start("c", cfg(), Some("nope"))).await.unwrap_err();
        assert_eq!(e.code, -32002);
        let d = p.describe().await.unwrap();
        assert_eq!(d.restore.exact, ExactKind::Save);
        assert!(!d.restore.approximate);
        assert!(d.fingerprint["server_version"].is_string());
        p.stop(IdParams {
            instance: "a".into(),
        })
        .await
        .unwrap();
        p.drop_save(SaveId { save: "s1".into() }).await.unwrap();
        let e = p
            .inspect(InspectParams {
                target: Target::Save { save: "s1".into() },
                keys: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, -32002);
    })
    .await;
}

#[tokio::test]
async fn diff_reports_changed_row() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        p.start(start("a", cfg(), None)).await.unwrap();
        exec(&p, "a", "INSERT INTO t(id, v) VALUES (1, 'x'), (2, 'y')").await;
        save(&p, "a", "s1").await;
        exec(&p, "a", "UPDATE t SET v = 'z' WHERE id = 1").await;
        let d = p
            .diff(DiffParams {
                a: Target::Instance {
                    instance: "a".into(),
                },
                b: Target::Save { save: "s1".into() },
            })
            .await
            .unwrap();
        assert_eq!(d.differing, vec!["t/1".to_string()]);
        let same = p
            .diff(DiffParams {
                a: Target::Save { save: "s1".into() },
                b: Target::Save { save: "s1".into() },
            })
            .await
            .unwrap();
        assert!(same.differing.is_empty());
        let docs = p
            .inspect(InspectParams {
                target: Target::Instance {
                    instance: "a".into(),
                },
                keys: vec!["t/1".into(), "t/9".into(), "nope/1".into()],
            })
            .await
            .unwrap()
            .docs;
        assert_eq!(docs["t/1"], json!({"id": 1, "v": "z"}));
        assert_eq!(docs["t/9"], Value::Null);
        assert_eq!(docs["nope/1"], Value::Null);
        let all = p
            .inspect(InspectParams {
                target: Target::Save { save: "s1".into() },
                keys: vec![],
            })
            .await
            .unwrap()
            .docs;
        assert_eq!(all.len(), 2);
        assert_eq!(all["t/2"], json!({"id": 2, "v": "y"}));
    })
    .await;
}

#[tokio::test]
async fn legacy_snap_name_accepted() {
    // Empty namespace, so legacy names are recognised; the database name is test-unique.
    let name = format!("snap_t5_{}_legacy", std::process::id());
    // every database the test makes is registered before anything can fail
    let extra = vec![name.clone(), format!("pi_t5_{}_li", std::process::id())];
    let nm = name.clone();
    with_cleanup("none_", &extra, move || async move {
        let t = Templates::new(&url());
        t.drop(&nm).await.unwrap();
        t.create_empty(&nm).await.unwrap();
        t.with_db(&nm, async |c| {
            c.batch_execute(
                "CREATE TABLE t(id int primary key, v text); INSERT INTO t VALUES (7, 'old')",
            )
            .await?;
            Ok(())
        })
        .await
        .unwrap();
        let p = Postgres::new(&url(), "").with_sql_tools();
        let docs = p
            .inspect(InspectParams {
                target: Target::Save { save: nm.clone() },
                keys: vec!["t/7".into()],
            })
            .await
            .unwrap()
            .docs;
        assert_eq!(docs["t/7"], json!({"id": 7, "v": "old"}));
        // and a new instance can start from it
        let inst = format!("t5_{}_li", std::process::id());
        p.start(StartParams {
            instance: inst.clone(),
            config: json!({}),
            seed: 1,
            clock_ms: 0,
            from: Some(FromSave {
                save: nm.clone(),
                mode: Mode::Exact,
            }),
        })
        .await
        .unwrap();
        let r = query(&p, &inst, "SELECT v FROM t").await;
        assert_eq!(r.result, Some(json!([{"v": "old"}])));
        p.stop(IdParams {
            instance: inst.clone(),
        })
        .await
        .unwrap();
        Templates::new(&url())
            .drop(&format!("pi_{inst}"))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn gc_keeps_listed() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        p.start(start("live", cfg(), None)).await.unwrap();
        p.start(start("orphan", cfg(), None)).await.unwrap();
        save(&p, "live", "keep").await;
        save(&p, "live", "toss").await;
        // a fresh process (as after a respawn) has no live instances, so it drops both
        let p2 = Postgres::new(&url(), &n);
        let r = p2
            .gc(GcParams {
                keep: vec!["keep".into()],
            })
            .await
            .unwrap();
        assert_eq!(r.dropped, 3, "{r:?}");
        assert!(r.bytes > 0);
        let t = Templates::new(&url());
        assert!(t.exists(&format!("ps_{n}keep")).await.unwrap());
        assert!(!t.exists(&format!("ps_{n}toss")).await.unwrap());
        assert!(!t.exists(&format!("pi_{n}live")).await.unwrap());
        assert!(!t.exists(&format!("pi_{n}orphan")).await.unwrap());
        // a live instance survives its own process's gc
        p.start(start("mine", cfg(), None)).await.unwrap();
        let r = p.gc(GcParams { keep: vec![] }).await.unwrap();
        assert_eq!(r.dropped, 1, "{r:?}");
        assert!(t.exists(&format!("pi_{n}mine")).await.unwrap());
    })
    .await;
}

fn build_all() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let a = arena();
        assert!(Command::new("cargo")
            .args(["build", "-p", "arena-postgres"])
            .current_dir(&a)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("cargo")
            .args(["build", "-p", "parts", "--bin", "arena-part-check"])
            .current_dir(a.join("core"))
            .status()
            .unwrap()
            .success());
    });
}

#[tokio::test]
async fn conformance_check_passes() {
    build_all();
    let ns = ns();
    let a = arena();
    let cfgfile = std::env::temp_dir().join(format!("{}pgcheck.json", ns));
    std::fs::write(&cfgfile, cfg().to_string()).unwrap();
    let saves = std::env::temp_dir().join(format!("{}saves", ns));
    let (cf, sv, ns2) = (cfgfile.clone(), saves.clone(), ns.clone());
    with_cleanup(&ns, &[], move || async move {
        let out = tokio::task::spawn_blocking(move || {
            Command::new(a.join("core/target/debug/arena-part-check"))
                .arg(a.join("integrations/postgres/check"))
                .arg("--config")
                .arg(&cf)
                .env("ARENA_BIN_DIR", a.join("core/target/debug"))
                .env("ARENA_SAVES", &sv)
                .env("ARENA_PG_URL", format!("{}?arena_ns={}", url(), ns2))
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success() && stdout.contains("\"passed\": true"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    })
    .await;
    let _ = std::fs::remove_file(&cfgfile);
    let _ = std::fs::remove_dir_all(&saves);
}

#[tokio::test]
async fn stop_drops_instance_and_no_tools_by_default() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n);
        assert!(p.describe().await.unwrap().tools.is_empty());
        assert_eq!(
            Postgres::new(&url(), &n)
                .with_sql_tools()
                .describe()
                .await
                .unwrap()
                .tools
                .len(),
            2
        );
        p.start(start("a", cfg(), None)).await.unwrap();
        let r = query(&p, "a", "SELECT 1").await;
        assert!(r.error.is_some(), "tools are off without --sql-tools");
        let t = Templates::new(&url());
        assert!(t.exists(&format!("pi_{n}a")).await.unwrap());
        p.stop(IdParams {
            instance: "a".into(),
        })
        .await
        .unwrap();
        assert!(!t.exists(&format!("pi_{n}a")).await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn diff_covers_tables_without_primary_key_and_other_schemas() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        let init = "CREATE TABLE np(a int, b text); INSERT INTO np VALUES (1, 'x'); \
                    CREATE SCHEMA other; CREATE TABLE other.o(id int primary key, v text); \
                    INSERT INTO other.o VALUES (1, 'x')";
        p.start(start("a", json!({"init_sql": init}), None))
            .await
            .unwrap();
        save(&p, "a", "s1").await;
        let d = |a: &str, b: &str| DiffParams {
            a: Target::Instance { instance: a.into() },
            b: Target::Save { save: b.into() },
        };
        assert!(p.diff(d("a", "s1")).await.unwrap().differing.is_empty());
        exec(&p, "a", "UPDATE np SET b = 'y'").await;
        assert_eq!(
            p.diff(d("a", "s1")).await.unwrap().differing,
            vec!["np/*".to_string()]
        );
        exec(&p, "a", "UPDATE np SET b = 'x'").await;
        exec(&p, "a", "UPDATE other.o SET v = 'y'").await;
        assert_eq!(
            p.diff(d("a", "s1")).await.unwrap().differing,
            vec!["other.o/*".to_string()]
        );
        let e = p
            .inspect(InspectParams {
                target: Target::Instance {
                    instance: "a".into(),
                },
                keys: vec!["np/1".into()],
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, -32602);
        assert!(
            e.message
                .contains("table np has no single-column primary key"),
            "{}",
            e.message
        );
    })
    .await;
}

#[tokio::test]
async fn null_and_typed_params() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        let init = "CREATE TABLE t(id int primary key, n int, ok boolean, j jsonb, v text)";
        p.start(start("a", json!({"init_sql": init}), None))
            .await
            .unwrap();
        let r = p
            .call_tool(CallToolParams {
                instance: "a".into(),
                name: "sql_execute".into(),
                args: json!({"sql": "INSERT INTO t(id, n, ok, j, v) VALUES ($1, $2, $3, $4, $5)",
                             "params": [1, null, true, null, null]}),
                clock_ms: 0,
            })
            .await
            .unwrap();
        assert_eq!(r.error, None);
        let r = query(&p, "a", "SELECT * FROM t").await;
        assert_eq!(
            r.result,
            Some(json!([{"id": 1, "n": null, "ok": true, "j": null, "v": null}]))
        );
    })
    .await;
}

#[tokio::test]
async fn save_closes_a_held_connection() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        p.start(start("a", cfg(), None)).await.unwrap();
        let (client, conn) = arena_part::helpers::pg::tokio_postgres::connect(
            &Templates::new(&url()).db_url(&format!("pi_{n}a")),
            arena_part::helpers::pg::tokio_postgres::NoTls,
        )
        .await
        .unwrap();
        let h = tokio::spawn(conn);
        client.simple_query("SELECT 1").await.unwrap();
        save(&p, "a", "s1").await;
        assert!(
            client.simple_query("SELECT 1").await.is_err(),
            "held connection should be closed"
        );
        let _ = h.await;
    })
    .await;
}

#[tokio::test]
async fn bad_id_is_invalid_params() {
    let p = Postgres::new(&url(), "none_");
    let e = p.start(start("Bad Id", json!({}), None)).await.unwrap_err();
    assert_eq!(e.code, -32602);
    let e = p
        .stop(IdParams {
            instance: "UPPER".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32602);
    let e = p
        .inspect(InspectParams {
            target: Target::Save { save: "A".into() },
            keys: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, -32602);
    // A valid id whose database name would not fit with the namespace.
    let e = p
        .start(start(&"x".repeat(60), json!({}), None))
        .await
        .unwrap_err();
    assert_eq!(e.code, -32602, "{}", e.message);
}

#[tokio::test]
async fn legacy_drop_save_drops_snap_and_branch_only() {
    // Empty namespace, so legacy names are recognised; the database names are test-unique.
    let pid = std::process::id();
    let names = [
        format!("snap_t5_{pid}_drop"),
        format!("branch_t5_{pid}_drop"),
        format!("gold_t5_{pid}_drop"),
    ];
    let todo = names.to_vec();
    with_cleanup("none_", &names, move || async move {
        let t = Templates::new(&url());
        for n in &todo {
            t.drop(n).await.unwrap();
            t.create_empty(n).await.unwrap();
        }
        let p = Postgres::new(&url(), "");
        for n in &todo {
            p.drop_save(SaveId { save: n.clone() }).await.unwrap();
        }
        assert!(!t.exists(&todo[0]).await.unwrap(), "snap_ is dropped");
        assert!(!t.exists(&todo[1]).await.unwrap(), "branch_ is dropped");
        assert!(t.exists(&todo[2]).await.unwrap(), "gold_ is kept");
        // A save that isn't there (any more) is -32002: arenad counts it as already gone.
        let gone = p
            .drop_save(SaveId {
                save: todo[0].clone(),
            })
            .await;
        assert_eq!(gone.unwrap_err().code, -32002);
    })
    .await;
}

#[tokio::test]
async fn gc_keep_prefix_keeps_matching_saves() {
    let ns = ns();
    let n = ns.clone();
    with_cleanup(&ns, &[], move || async move {
        let p = Postgres::new(&url(), &n).with_sql_tools();
        p.start(start("i", cfg(), None)).await.unwrap();
        save(&p, "i", "gold_x").await;
        save(&p, "i", "s_toss").await;
        p.stop(IdParams {
            instance: "i".into(),
        })
        .await
        .unwrap();
        let r = p
            .gc(GcParams {
                keep: vec!["gold_*".into()],
            })
            .await
            .unwrap();
        assert_eq!(r.dropped, 1, "{r:?}");
        let t = Templates::new(&url());
        assert!(t.exists(&format!("ps_{n}gold_x")).await.unwrap());
        assert!(!t.exists(&format!("ps_{n}s_toss")).await.unwrap());
    })
    .await;
}
