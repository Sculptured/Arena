use std::{future::Future, sync::LazyLock};

use arenad::{model::*, store::Store};
use serde_json::json;
use world::{pg::Admin, testing};

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

async fn fresh(tag: &str) -> (Admin, String, Store, testing::Cleanup) {
    let admin = testing::admin();
    let name = db_name(tag);
    let clean = testing::Cleanup::new();
    clean.meta(&name);
    let store = Store::open_named(&admin, &name).await.unwrap();
    (admin, name, store, clean)
}

/// The `Cleanup` guard returned by `fresh` drops the database when the test ends.
async fn done(_admin: Admin, _name: String, store: Store) {
    drop(store);
}

#[test]
fn append_assigns_increasing_seq() {
    run(async {
        let (admin, name, store, _g) = fresh("append").await;
        let run_id = store.create_run("task", &team()).await.unwrap();
        let b = store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
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
        let mut hs = vec![];
        for i in 0..20 {
            let (s, b) = (store.clone(), b.clone());
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
        assert_eq!(seqs, (3..=22).collect::<Vec<_>>());
        let evs = store.events(&b).await.unwrap();
        assert_eq!(evs.len(), 22);
        assert_eq!(evs[1].agent.as_deref(), Some("a"));
        assert_eq!(evs[1].kind, EventKind::Handoff);

        let br = store.branch(&b).await.unwrap();
        assert_eq!(br.status, BranchStatus::Running);
        assert!(br.ended_at.is_none());
        store
            .set_status(&b, BranchStatus::Passed, None)
            .await
            .unwrap();
        let br = store.branch(&b).await.unwrap();
        assert_eq!(br.status, BranchStatus::Passed);
        assert!(br.ended_at.is_some());
        assert_eq!(br.team, team());
        let runs = store.runs().await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].branches.len(), 1);

        done(admin, name, std::sync::Arc::into_inner(store).unwrap()).await;
    })
}

#[test]
fn copy_prefix_keeps_seqs_and_new_events_continue_after() {
    run(async {
        let (admin, name, store, _g) = fresh("copy").await;
        let run_id = store.create_run("task", &team()).await.unwrap();
        let parent = store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
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
        let copied = store.events(&child).await.unwrap();
        assert_eq!(
            copied.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let e = store
            .append(&child, None, EventKind::ToolCall, json!({}))
            .await
            .unwrap();
        assert_eq!(e.seq, 4);
        assert_eq!(store.events(&parent).await.unwrap().len(), 5);
        let b = store.branch(&child).await.unwrap();
        assert_eq!(b.parent_id.as_deref(), Some(parent.as_str()));
        assert_eq!(b.start_seq, Some(4));
        assert_eq!(b.fork_note.as_deref(), Some("note"));
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
