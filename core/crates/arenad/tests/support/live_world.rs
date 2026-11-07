//! A whole world of real services for a live test: an ERPNext stack, a Twenty server and saves of
//! the test's own (removed when the `Stack` drops), the Sculptured program, a team on a real model
//! run to its check, and what Arena owes any such run.
#![allow(dead_code)]

use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

use arenad::{app::AppState, model::*, runner, scenario::Scenario};

use crate::packages::arena;
use crate::pg::pg_url;

/// The ERPNext stack and server, the saves and the sites of this test; removed on drop.
pub struct Stack {
    id: String,
    pub saves: PathBuf,
}

impl Stack {
    /// A stack whose containers, projects and saves carry `tag` and the test's process id.
    pub fn new(tag: &str) -> Stack {
        let id = format!("{tag}{}", std::process::id());
        let saves = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("world-{id}"));
        std::fs::create_dir_all(&saves).unwrap();
        Stack { id, saves }
    }

    pub fn env(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("ARENA_PG_URL".to_string(), pg_url()),
            ("PATH".to_string(), path_with_sculptured()),
        ];
        let port = |n: u16| (n + (std::process::id() % 1000) as u16).to_string();
        for (k, v) in [
            (
                "ARENA_ERPNEXT_DB_CONTAINER",
                format!("arena-erpnext-db-{}", self.id),
            ),
            (
                "ARENA_ERPNEXT_DB_VOLUME",
                format!("arena-erpnext-db-{}", self.id),
            ),
            ("ARENA_ERPNEXT_DB_PORT", port(23000)),
            (
                "ARENA_ERPNEXT_PROJECT",
                format!("arena_erpnext_{}", self.id),
            ),
            ("ARENA_ERPNEXT_PORT", port(24000)),
            ("ARENA_TWENTY_PROJECT", format!("arena_twenty_{}", self.id)),
            ("ARENA_TWENTY_DB_PORT", port(25000)),
        ] {
            out.push((k.to_string(), v));
        }
        out
    }
}

impl Stack {
    /// The CRM's servers, then its database server, network and volume.
    fn remove_crm(&self) {
        let name = format!("arena-twenty-{}", self.id);
        let docker = |args: &[&str]| {
            let out = Command::new("docker").args(args).output();
            out.map_or(String::new(), |o| {
                String::from_utf8_lossy(&o.stdout).into_owned()
            })
        };
        for c in docker(&["ps", "-aq", "--filter", &format!("name=^{name}")]).split_whitespace() {
            docker(&["rm", "-f", "-v", c]);
        }
        let project = format!("name=arena_twenty_{}", self.id);
        for n in docker(&["network", "ls", "-q", "--filter", &project]).split_whitespace() {
            docker(&["network", "rm", n]);
        }
        docker(&["network", "rm", &name]);
        docker(&["volume", "rm", "-f", &format!("{name}-db")]);
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let db = format!("arena-erpnext-db-{}", self.id);
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", &db])
            .output();
        let compose = arena().join("integrations/erpnext/compose.yml");
        let project = format!("arena_erpnext_{}", self.id);
        let mut down = Command::new("docker");
        down.args(["compose", "-p", &project, "-f"]).arg(&compose);
        down.env("ARENA_ERPNEXT_SITES", self.saves.join("erpnext/.sites"));
        let _ = down.args(["down", "-v"]).output();
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &db])
            .output();
        self.remove_crm();
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

/// `PATH` with the Sculptured program to use first: `ARENA_SCULPTURED_BINARY`, else a release build
/// next to the arena checkout, else the one already on it.
pub fn path_with_sculptured() -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let local = arena().join("../sculptured/target/release/sculptured");
    let set = std::env::var_os("ARENA_SCULPTURED_BINARY").map(PathBuf::from);
    let binary = set.or_else(|| local.exists().then_some(local));
    match binary.as_deref().and_then(|b| b.parent()) {
        Some(dir) => format!("{}:{path}", dir.display()),
        None => path,
    }
}

/// Runs the team on `task` to its end, polling every 200 ms up to `polls` times; the branch,
/// the checks that failed, and the branch's events.
pub async fn walk_for(
    state: &Arc<AppState>,
    task: &str,
    team: Team,
    polls: usize,
) -> (Branch, CheckPayload, Vec<Event>) {
    walk_in(state, task, team, &Scenario::default(), polls).await
}

/// `walk_for` in `scenario`.
pub async fn walk_in(
    state: &Arc<AppState>,
    task: &str,
    team: Team,
    scenario: &Scenario,
    polls: usize,
) -> (Branch, CheckPayload, Vec<Event>) {
    let started = runner::start_run_in(state, task, team, Runner::Chain, scenario);
    let (_run, id) = started.await.unwrap();
    for _ in 0..polls {
        let b = state.store.branch(&id).await.unwrap();
        if b.status != BranchStatus::Running {
            let evs = state.store.events(&id).await.unwrap();
            let check = evs.iter().find(|e| e.kind == EventKind::Check);
            let payload = check
                .unwrap_or_else(|| panic!("no check: {b:?}"))
                .payload
                .clone();
            return (b, serde_json::from_value(payload).unwrap(), evs);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the branch did not finish");
}

/// A scored benchmark's check, whatever the team did: every named check is there, the differing
/// rows are the names of the checks that failed, and the branch passed when every check did and
/// failed when a goal (not a safety check) was missed.
pub fn assert_scored(b: &Branch, check: &CheckPayload) {
    let checks = check.checks.clone().expect("named checks");
    assert!(!checks.is_empty());
    let mut failed: Vec<String> = checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name.clone())
        .collect();
    let mut rows = check.differing_rows.clone().unwrap_or_default();
    failed.sort();
    rows.sort();
    assert_eq!(rows, failed, "{check:?}");
    if checks.iter().all(|c| c.passed) {
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
    }
    if checks.iter().any(|c| !c.passed && !c.safety) {
        assert_eq!(b.status, BranchStatus::Failed, "{:?}", b.error);
    }
    let passed = checks.iter().filter(|c| c.passed).count();
    println!(
        "checks: {passed} of {} passed; failed: {failed:?}",
        checks.len()
    );
}

/// The gates held: a gated tool's call naming a change no one had proposed was refused before it
/// reached its part.
pub fn assert_gates_kept(bench: &parts::proto::BenchDescribe, evs: &[Event]) {
    let gated: Vec<&str> = bench.gates.iter().map(|g| g.tool.as_str()).collect();
    let mut proposed: Vec<String> = vec![];
    let (mut through, mut refused) = (0, 0);
    for e in evs.iter().filter(|e| e.kind == EventKind::ToolCall) {
        let p = &e.payload;
        let ok = p["error"].is_null();
        if p["name"] == "propose_change" && ok {
            if let Some(id) = p["result"]["id"].as_str() {
                proposed.push(id.to_string());
            }
        }
        if !gated.contains(&p["name"].as_str().unwrap_or_default()) {
            continue;
        }
        match ok {
            true => through += 1,
            false => refused += 1,
        }
        if let Some(id) = p["args"]["proposal_id"].as_str() {
            if !proposed.iter().any(|x| x == id) {
                assert!(
                    !ok && p["part"].is_null(),
                    "{} on {id}, which no one proposed: {p}",
                    p["name"]
                );
            }
        }
    }
    println!("gated calls: {through} went through, {refused} refused");
}
