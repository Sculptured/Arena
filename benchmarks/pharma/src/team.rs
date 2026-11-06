//! The incident's ticket, the rules every agent gets and the deployments this benchmark suggests
//! (`data/teams.json`): a single agent and a coordinated team whose roles have only the tools
//! their decisions need, with short prompts and compact reports. Arena imposes none of it.
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
    pub roles: Vec<Role>,
}

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
    found.expect("data/teams.json has the single agent and the team")
}

/// `id`'s deployment as arenad's team: every role but the last hands off.
pub fn bench_team(id: &str) -> BenchTeam {
    let d = deployment(id);
    let last = d.roles.len().saturating_sub(1);
    let agent = |(i, r): (usize, &Role)| TeamRole {
        id: r.id.clone(),
        name: r.name.clone(),
        prompt: format!(
            "You are the {} at Larchmont Biologics. {}{}",
            r.name,
            r.prompt,
            if i < last { HANDOFF } else { "" }
        ),
        read_only: r.read_only,
    };
    BenchTeam {
        id: d.id.clone(),
        name: d.name.clone(),
        agents: d.roles.iter().enumerate().map(agent).collect(),
    }
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
    fn no_role_decides_and_the_single_agent_has_every_tool() {
        let roles = &deployment("team").roles;
        let all: Vec<&str> = roles.iter().flat_map(tools_of).collect();
        assert!(!all.contains(&"decide_change"));
        let single = tools_of(&deployment("single").roles[0]);
        let missing: Vec<&&str> = all.iter().filter(|t| !single.contains(t)).collect();
        assert!(missing.is_empty(), "the single agent lacks {missing:?}");
    }
}
