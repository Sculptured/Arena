use std::{future::Future, sync::LazyLock};

use arenad::{model::*, pg::Admin, store::Store};
use serde_json::json;

#[path = "support/pg.rs"]
mod pg;

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![AgentSpec {
            id: "a".into(),
            name: "A".into(),
            kind: AgentKind::Open,
            model: None,
            prompt: "p".into(),
            provider: None,
        }],
    }
}

fn db_name(tag: &str) -> String {
    format!(
        "arena_test_{}_{}",
        tag,
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

async fn fresh(tag: &str) -> (Admin, String, Store, pg::Cleanup) {
    let admin = pg::admin();
    let name = db_name(tag);
    let clean = pg::Cleanup::new();
    clean.add(&name);
    let store = Store::open_named(&admin, &name).await.unwrap();
    (admin, name, store, clean)
}

/// The `Cleanup` guard returned by `fresh` drops the database when the test ends.
async fn done(_admin: Admin, _name: String, store: Store) {
    drop(store);
}

/// A run of "task" with one root branch; returns the run id and the branch id.
async fn root_branch(store: &Store) -> (String, String) {
    let run_id = store.create_run("task", &team()).await.unwrap();
    let b = store
        .create_branch(&run_id, None, &team(), &json!([]), None)
        .await
        .unwrap();
    (run_id, b)
}

/// Appends `n` tool calls to `b` from concurrent tasks and returns their seqs, sorted.
async fn append_concurrently(store: &std::sync::Arc<Store>, b: &str, n: usize) -> Vec<i64> {
    let mut hs = vec![];
    for i in 0..n {
        let (s, b) = (store.clone(), b.to_string());
        hs.push(tokio::spawn(async move {
            s.append(&b, None, EventKind::ToolCall, json!({ "i": i }))
                .await
                .unwrap()
                .seq
        }));
    }
    let mut seqs = vec![];
    for h in hs {
        seqs.push(h.await.unwrap());
    }
    seqs.sort();
    seqs
}

/// A running branch becomes passed, with an end time, and the run lists it.
async fn assert_passes_and_is_listed(store: &Store, b: &str) {
    let br = store.branch(b).await.unwrap();
    assert_eq!(br.status, BranchStatus::Running);
    assert!(br.ended_at.is_none());
    store
        .set_status(b, BranchStatus::Passed, None)
        .await
        .unwrap();
    let br = store.branch(b).await.unwrap();
    assert_eq!(br.status, BranchStatus::Passed);
    assert!(br.ended_at.is_some());
    assert_eq!(br.team, team());
    let runs = store.runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].branches.len(), 1);
}

#[test]
fn append_assigns_increasing_seq() {
    run(async {
        let (admin, name, store, _g) = fresh("append").await;
        let (_, b) = root_branch(&store).await;
        let mut rx = store.subscribe();
        let e1 = store
            .append(&b, None, EventKind::RunStart, json!({}))
            .await
            .unwrap();
        let e2 = store
            .append(&b, Some("a"), EventKind::Handoff, json!({"note": "x"}))
            .await
            .unwrap();
        assert_eq!((e1.seq, e2.seq), (1, 2));
        assert_eq!(rx.recv().await.unwrap().seq, 1);
        assert_eq!(rx.recv().await.unwrap().seq, 2);

        // Concurrent appenders on one branch get distinct, gap-free seqs.
        let store = std::sync::Arc::new(store);
        let seqs = append_concurrently(&store, &b, 20).await;
        assert_eq!(seqs, (3..=22).collect::<Vec<_>>());
        let evs = store.events(&b).await.unwrap();
        assert_eq!(evs.len(), 22);
        assert_eq!(evs[1].agent.as_deref(), Some("a"));
        assert_eq!(evs[1].kind, EventKind::Handoff);

        assert_passes_and_is_listed(&store, &b).await;

        done(admin, name, std::sync::Arc::into_inner(store).unwrap()).await;
    })
}

/// `child` holds copies of `parent`'s events 1..=3 and its next event is 4; `parent` is untouched.
async fn assert_child_continues_after_prefix(store: &Store, parent: &str, child: &str) {
    let copied = store.events(child).await.unwrap();
    assert_eq!(
        copied.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let e = store
        .append(child, None, EventKind::ToolCall, json!({}))
        .await
        .unwrap();
    assert_eq!(e.seq, 4);
    assert_eq!(store.events(parent).await.unwrap().len(), 5);
    let b = store.branch(child).await.unwrap();
    assert_eq!(b.parent_id.as_deref(), Some(parent));
    assert_eq!(b.start_seq, Some(4));
    assert_eq!(b.fork_note.as_deref(), Some("note"));
}

#[test]
fn copy_prefix_keeps_seqs_and_new_events_continue_after() {
    run(async {
        let (admin, name, store, _g) = fresh("copy").await;
        let (run_id, parent) = root_branch(&store).await;
        for i in 0..5 {
            store
                .append(&parent, None, EventKind::ToolCall, json!({ "i": i }))
                .await
                .unwrap();
        }
        let child = store
            .create_branch(
                &run_id,
                Some((&parent, 4)),
                &team(),
                &json!([{"prompt": "x"}]),
                Some("note".into()),
            )
            .await
            .unwrap();
        store.copy_prefix(&parent, &child, 4).await.unwrap();
        assert_child_continues_after_prefix(&store, &parent, &child).await;
        done(admin, name, store).await;
    })
}

#[test]
fn open_marks_running_branches_interrupted() {
    run(async {
        let (admin, name, store, _g) = fresh("interrupt").await;
        let run_id = store.create_run("task", &team()).await.unwrap();
        let running = store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
        let passed = store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
        store
            .set_status(&passed, BranchStatus::Passed, None)
            .await
            .unwrap();
        drop(store);

        let store = Store::open_named(&admin, &name).await.unwrap();
        let r = store.branch(&running).await.unwrap();
        assert_eq!(r.status, BranchStatus::Interrupted);
        assert!(r.ended_at.is_some());
        assert_eq!(
            store.branch(&passed).await.unwrap().status,
            BranchStatus::Passed
        );
        done(admin, name, store).await;
    })
}
