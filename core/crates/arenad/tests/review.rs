//! AI review: its storage (on Postgres), and the reviewer itself, live, on a real model.
//!
//! The live test reviews a recorded run (the single Claude Code Sonnet agent of the pharma demo,
//! from the review module's test data) with Claude Code on `ARENA_LIVE_MODEL` (default `haiku`)
//! and checks that the answer is a valid review whose every cited step is a step of that run. It
//! prints `skipped: …` and passes without `claude`.

use std::{future::Future, process::Command, sync::LazyLock};

use arenad::{
    model::*,
    review::{
        self,
        digest::{digest, steps_of, RunRecord, MAX_DIGEST_BYTES},
        output::Citable,
        prompt,
        reviewer::ask,
        settings::ReviewerModel,
        Job, Shape,
    },
    store::{ReviewDone, Store},
};
use serde_json::{json, Value};

#[path = "support/pg.rs"]
mod pg;

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn db_name() -> String {
    format!(
        "arena_test_review_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

#[test]
fn reviews_are_stored_once_kept_and_replaced_only_when_asked() {
    run(async {
        let admin = pg::admin();
        let name = db_name();
        let clean = pg::Cleanup::new();
        clean.add(&name);
        let store = Store::open_named(&admin, &name).await.unwrap();
        let key = review::branch_key("b1");
        assert!(store.review(&key).await.unwrap().is_none());
        let row = store
            .start_review(&key, "run", "claude_code:haiku")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (row.status.as_str(), row.model.as_str()),
            ("running", "claude_code:haiku")
        );
        // One review of a key at a time.
        assert!(store
            .start_review(&key, "run", "claude_code:haiku")
            .await
            .unwrap()
            .is_none());
        finish_and_read(&store, &key).await;
        replace_and_fail(&store, &key).await;
        restart_marks_running_failed(store, &admin, &name).await;
    });
}

async fn finish_and_read(store: &Store, key: &str) {
    let answer = json!({"verdict": "v", "went_wrong": []});
    let issues = vec!["dropped citation 9: no such step".to_string()];
    let done = ReviewDone {
        review: &answer,
        issues: &issues,
        cost_usd: Some(0.02),
        cost_source: Some("api_equivalent_estimate"),
        tokens: Some(1200),
        seconds: 12.5,
        digest_bytes: 9000,
    };
    store.finish_review(key, done).await.unwrap();
    let got = store.review(key).await.unwrap().unwrap();
    assert_eq!(got.status, "done");
    assert_eq!(got.review, Some(answer));
    assert_eq!(got.issues, json!(issues));
    assert_eq!(
        (got.cost_usd, got.tokens, got.digest_bytes),
        (Some(0.02), Some(1200), Some(9000))
    );
    assert!(got.finished_at.is_some());
}

async fn replace_and_fail(store: &Store, key: &str) {
    // Asked again: replaced, the old answer gone.
    let again = store
        .start_review(key, "run", "claude_code:sonnet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (again.status.as_str(), again.review.clone()),
        ("running", None)
    );
    store
        .fail_review(key, "the reviewer did not answer")
        .await
        .unwrap();
    let failed = store.review(key).await.unwrap().unwrap();
    assert_eq!(failed.error.as_deref(), Some("the reviewer did not answer"));
}

async fn restart_marks_running_failed(store: Store, admin: &arenad::pg::Admin, name: &str) {
    // A review running when arenad stops is marked failed when it starts again.
    let other = review::compare_key("a", "b");
    store
        .start_review(&other, "compare", "m")
        .await
        .unwrap()
        .unwrap();
    drop(store);
    let reopened = Store::open_named(admin, name).await.unwrap();
    let r = reopened.review(&other).await.unwrap().unwrap();
    assert_eq!(r.status, "error");
    assert!(r.error.unwrap().contains("arenad stopped"));
    drop(reopened);
}

fn claude_installed() -> bool {
    Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn recorded_run_job() -> (Vec<Event>, Job) {
    let v: Value =
        serde_json::from_str(include_str!("../src/review/testdata/single_sonnet.json")).unwrap();
    let branch: Branch = serde_json::from_value(v.clone()).unwrap();
    let events: Vec<Event> = serde_json::from_value(v["events"].clone()).unwrap();
    let rec = RunRecord {
        branch: &branch,
        events: &events,
        ticket: None,
    };
    let job = Job {
        prompt: prompt::run_prompt(&digest(&rec, MAX_DIGEST_BYTES)),
        citable: Citable {
            steps_a: steps_of(&events),
            agents: branch.team.agents.iter().map(|a| a.id.clone()).collect(),
            ..Citable::default()
        },
        shape: Shape::Run,
    };
    (events, job)
}

fn cited_steps(review: &Value) -> Vec<i64> {
    review["went_wrong"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| {
            w["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_i64)
        })
        .collect()
}

#[test]
#[ignore]
fn live_review_of_a_recorded_run_is_valid_and_cites_real_steps() {
    if !claude_installed() {
        println!("skipped: claude is not installed");
        return;
    }
    let model = std::env::var("ARENA_LIVE_MODEL").unwrap_or_else(|_| "haiku".into());
    let (events, job) = recorded_run_job();
    let reviewer = ReviewerModel::ClaudeCode { model };
    let cfg = arenad::providers::ProviderConfig::from_env();
    let http = reqwest::Client::new();
    let answer = run(ask(&reviewer, &job.prompt, &cfg, &http)).expect("the reviewer answers");
    let (review, issues) = review::check(&answer.value, &job).expect("a valid review");
    println!(
        "review ({:?} USD, {:?} tokens, {:.0} s): {review:#}",
        answer.cost_usd, answer.tokens, answer.seconds
    );
    println!("issues: {issues:?}");
    assert!(answer.cost_usd.is_some_and(|c| c > 0.0));
    assert!(!review["verdict"].as_str().unwrap().is_empty());
    let cited = cited_steps(&review);
    let steps = steps_of(&events);
    assert!(cited.iter().all(|s| steps.contains(s)));
    // The run failed one check: the review says something went wrong, citing steps.
    assert!(!review["went_wrong"].as_array().unwrap().is_empty());
    if cited.is_empty() {
        println!("note: the reviewer cited no step that exists");
    }
}
