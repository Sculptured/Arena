//! The incident's ticket, the rules every agent gets and the deployments this benchmark suggests
//! (`data/teams.json`): a single agent (arenad's default), a dispatching team whose lead delegates
//! to the roles (for arenad's `dispatch` runner), and the coordinated team for a chain, whose roles
//! have only the tools their decisions need, with short prompts and compact reports. Arena imposes
//! none of it.
use std::sync::OnceLock;

use arena_part::proto::{BenchTeam, TeamRole};
use serde::Deserialize;

pub const INSTRUCTIONS_TITLE: &str = "Incident rules";

pub const TICKET: &str = "# CVX-2611: Grade A environmental-monitoring excursion\n\nLarchmont Biologics, \
US Plant, Monday 2026-11-09. During this morning's fill of Corvexa batch CVX-2611 on Filling Line 1, a \
Grade A environmental-monitoring sample exceeded its action limit (see the LIMS). Handle the incident: \
open a deviation, hold the batch, serve this week's orders planned from CVX-2611 from other eligible \
stock, bring the next batch forward, ship cold-chain, and tell each affected customer its delivery date, \
recorded in the CRM.\n";

pub const INSTRUCTIONS: &str = "- A person decides changes. Propose one with propose_change {action, \
summary, payload}; list_changes shows the decision and its note. Actions: deviation {batch, sample}, \
batch_hold {batch}, batch_release {batch}, stock_reallocation {batch, qty, orders}, cold_chain_shipment \
{carrier, quote_id, order, batch, qty}.
- In the ERP a batch's QA status (Batch field qa_status) is Quarantined, Released, On Hold or Rejected. \
Set Released only after the batch's batch_release is approved.
- A deviation is a Non Conformance in the ERP naming the batch and the sample (procedure \"EM excursion \
investigation (SOP-QA-031)\").
- Allocate by SOP-SC-014 (LIMS documents): ship each order whole from one lot, from that lot's site, on a \
lane qualified for 2-8 C. book_shipment needs the approved cold_chain_shipment.
- Tell each affected customer each order's delivery date (the shipment's arrival, or when it can be \
served) with send_customer_notice; in the CRM, log a note on the account and move the order's \
opportunity (its name starts with the PO number) to that date.
- Reports (affected_orders, stock_report, eligible_lots, production_schedule) summarise the ERP; \
list_docs and get_doc query it directly. Dates are YYYY-MM-DD.
- Never invent a number, an id or a reply.";

const HANDOFF: &str =
    " Finish by handing off what the next roles need: facts, numbers and change ids.";

#[derive(Debug, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub read_only: bool,
    pub prompt: String,
    /// Groups of `Teams::tools`.
    pub tools: Vec<String>,
    pub proposes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub name: String,
    /// The arenad runner its prompts are written for (absent: any).
    #[serde(default)]
    pub runner: Option<String>,
    pub roles: Vec<Role>,
}

/// The deployments by id, the default first.
pub const DEPLOYMENTS: [&str; 3] = ["single", "dispatch", "team"];

#[derive(Debug, Deserialize)]
pub struct Teams {
    pub tools: std::collections::BTreeMap<String, Vec<String>>,
    pub deployments: Vec<Deployment>,
}

pub fn teams() -> &'static Teams {
    static TEAMS: OnceLock<Teams> = OnceLock::new();
    TEAMS.get_or_init(|| {
        serde_json::from_str(include_str!("../data/teams.json"))
            .expect("data/teams.json has the deployments' shape (a test reads it)")
    })
}

/// The tools of `role`, its groups expanded.
pub fn tools_of(role: &Role) -> Vec<&'static str> {
    let groups = role.tools.iter().filter_map(|g| teams().tools.get(g));
    groups.flatten().map(String::as_str).collect()
}

fn deployment(id: &str) -> &'static Deployment {
    let found = teams().deployments.iter().find(|d| d.id == id);
    found.expect("data/teams.json has every deployment of DEPLOYMENTS")
}

/// `id`'s deployment as arenad's team: in a chain every role but the last hands off.
pub fn bench_team(id: &str) -> BenchTeam {
    let d = deployment(id);
    let last = d.roles.len().saturating_sub(1);
    let chain = d.runner.as_deref() == Some("chain");
    let agent = |(i, r): (usize, &Role)| TeamRole {
        id: r.id.clone(),
        name: r.name.clone(),
        prompt: format!(
            "You are the {} at Larchmont Biologics. {}{}",
            r.name,
            r.prompt,
            if chain && i < last { HANDOFF } else { "" }
        ),
        read_only: r.read_only,
    };
    BenchTeam {
        id: d.id.clone(),
        name: d.name.clone(),
        agents: d.roles.iter().enumerate().map(agent).collect(),
        runner: d.runner.clone(),
    }
}

/// Every deployment but `default`, as the other teams arenad offers.
pub fn other_teams(default: &str) -> Vec<BenchTeam> {
    let others = DEPLOYMENTS.iter().filter(|id| **id != default);
    others.map(|id| bench_team(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_reads_through_one_records_role() {
        let t = bench_team("team");
        let ids: Vec<&str> = t.agents.iter().map(|a| a.id.as_str()).collect();
        let want = [
            "coordinator",
            "records",
            "quality",
            "supply",
            "production",
            "logistics",
        ];
        assert_eq!(ids, want);
        let roles = &deployment("team").roles;
        let querying: Vec<&str> = (roles.iter())
            .filter(|r| tools_of(r).contains(&"list_docs"))
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(querying, ["records"]);
        assert!(
            t.agents.iter().all(|a| a.prompt.len() < 500),
            "prompts stay short"
        );
        assert!(!t.agents.last().unwrap().prompt.contains("handing off"));
        assert!(INSTRUCTIONS.len() < 2000 && TICKET.len() < 700);
    }

    #[test]
    fn the_dispatching_team_is_led_by_the_coordinator_and_never_hands_off() {
        let t = bench_team("dispatch");
        assert_eq!(t.runner.as_deref(), Some("dispatch"));
        let ids: Vec<&str> = t.agents.iter().map(|a| a.id.as_str()).collect();
        let chain: Vec<String> = bench_team("team")
            .agents
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(
            ids, chain,
            "the coordinated roles, the coordinator first (it leads)"
        );
        assert!(t.agents[0].prompt.contains("delegate") && t.agents[0].read_only);
        for a in &t.agents {
            let p = a.prompt.to_lowercase();
            assert!(
                !p.contains("hand off") && !p.contains("handing off"),
                "{}",
                a.id
            );
            assert!(a.prompt.len() < 800, "{} stays short", a.id);
        }
        assert!(t.agents[1..].iter().all(|a| a.prompt.contains("Report")));
        let others: Vec<String> = other_teams("single").into_iter().map(|t| t.id).collect();
        assert_eq!(others, ["dispatch", "team"]);
        assert_eq!(bench_team("single").runner, None);
    }

    #[test]
    fn no_role_decides_and_the_single_agent_has_every_tool() {
        let roles = &deployment("team").roles;
        let all: Vec<&str> = roles.iter().flat_map(tools_of).collect();
        assert!(!all.contains(&"decide_change"));
        let single = tools_of(&deployment("single").roles[0]);
        let missing: Vec<&&str> = all.iter().filter(|t| !single.contains(t)).collect();
        assert!(missing.is_empty(), "the single agent lacks {missing:?}");
    }
}
