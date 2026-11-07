//! A whole world of real services for a live test: an ERPNext stack, a Twenty server and saves of
//! the test's own (removed when the `Stack` drops), the Sculptured program, and a scripted team
//! walked through a run to its check.
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

pub fn scripted_team(hubteam: parts::proto::BenchTeam) -> Team {
    let agent = |a: parts::proto::TeamRole| AgentSpec {
        id: a.id,
        name: a.name,
        kind: AgentKind::Open,
        model: Some("scripted".into()),
        prompt: a.prompt,
        provider: None,
        read_only: a.read_only,
    };
    Team {
        id: hubteam.id,
        name: hubteam.name,
        agents: hubteam.agents.into_iter().map(agent).collect(),
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
