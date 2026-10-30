use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Instant,
};

use serde_json::json;
use tokio::sync::OnceCell;
use world::{
    dump,
    schema::load_base,
    snapshot::{Source, Worlds},
    testing::{self, Cleanup},
    tools,
};

// One runtime for every test: Admin's connection task lives on the runtime that first
// used it, so per-test runtimes would kill it when that test finishes.
static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

static WORLDS: OnceCell<Arc<Worlds>> = OnceCell::const_new();

async fn worlds() -> Arc<Worlds> {
    WORLDS
        .get_or_init(|| async {
            let w = Worlds::new(testing::admin());
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail");
            load_base(w.admin(), &dir).await.unwrap();
            Arc::new(w)
        })
        .await
        .clone()
}

fn pid_id(tag: &str) -> String {
    format!("snaptest_{}_{}", std::process::id(), tag)
}

async fn write(w: &Worlds, db: &str, n: usize) {
    let args = json!({"user_id": "ethan_garcia_1261", "address1": format!("{n} Highway"),
        "address2": "", "city": "New York", "state": "NY", "country": "USA", "zip": "10001"});
    let out = w
        .with_client(db, async |c| {
            tools::call(c, "modify_user_address", &args).await
        })
        .await
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(!out.writes.is_empty());
}

#[test]
fn snapshot_then_restore_round_trips() {
    run(async {
        let w = worlds().await;
        let g = Cleanup::new();
        g.branches_containing(&pid_id("rt"));
        let b = w.new_branch(&pid_id("rt"), Source::Base).await.unwrap();
        write(&w, &b, 1).await;
        let snap = w.snapshot(&b).await.unwrap();
        g.add(&snap);
        let expected = w.with_client(&b, async |c| dump(c).await).await.unwrap();
        write(&w, &b, 2).await;
        let after = w.with_client(&b, async |c| dump(c).await).await.unwrap();
        assert_ne!(after, expected);
        let b2 = w
            .new_branch(&pid_id("rt2"), Source::Snapshot(snap.clone()))
            .await
            .unwrap();
        let got = w.with_client(&b2, async |c| dump(c).await).await.unwrap();
        assert_eq!(got, expected);
        for d in [b, b2, snap] {
            w.drop_db(&d).await.unwrap();
        }
    })
}

#[test]
fn concurrent_snapshots_of_two_branches_succeed() {
    run(async {
        let w = worlds().await;
        let g = Cleanup::new();
        g.branches_containing(&pid_id("ca"));
        g.branches_containing(&pid_id("cb"));
        let a = w.new_branch(&pid_id("ca"), Source::Base).await.unwrap();
        let b = w.new_branch(&pid_id("cb"), Source::Base).await.unwrap();
        async fn cycles(w: &Worlds, g: &Cleanup, db: &str) -> anyhow::Result<Vec<String>> {
            let mut snaps = vec![];
            for n in 0..10 {
                let snap = w.snapshot(db).await?;
                g.add(&snap);
                snaps.push(snap);
                let args = json!({"user_id": "ethan_garcia_1261", "address1": format!("{n} Road"),
                "address2": "", "city": "New York", "state": "NY", "country": "USA", "zip": "10001"});
                w.with_client(db, async |c| {
                    tools::call(c, "modify_user_address", &args).await
                })
                .await?;
            }
            Ok(snaps)
        }
        let (ra, rb) = tokio::join!(cycles(&w, &g, &a), cycles(&w, &g, &b));
        let mut all = vec![a, b];
        all.extend(ra.unwrap());
        all.extend(rb.unwrap());
        for d in all {
            w.drop_db(&d).await.unwrap();
        }
    })
}

#[test]
fn snapshot_time_is_reported() {
    run(async {
        let w = worlds().await;
        let g = Cleanup::new();
        g.branches_containing(&pid_id("tm"));
        let b = w.new_branch(&pid_id("tm"), Source::Base).await.unwrap();
        let mut times = vec![];
        let mut snaps = vec![];
        for _ in 0..10 {
            let t = Instant::now();
            let snap = w.snapshot(&b).await.unwrap();
            g.add(&snap);
            snaps.push(snap);
            times.push(t.elapsed());
        }
        times.sort();
        println!("median snapshot copy time: {:?}", times[times.len() / 2]);
        for d in snaps.into_iter().chain([b]) {
            w.drop_db(&d).await.unwrap();
        }
    })
}

#[test]
fn bad_names_are_rejected() {
    run(async {
        let w = worlds().await;
        assert!(w
            .new_branch("x\"; DROP DATABASE postgres; --", Source::Base)
            .await
            .is_err());
        assert!(w.snapshot("Bad-Name").await.is_err());
        assert!(w.with_client("a b", async |_| Ok(())).await.is_err());
    })
}
