//! The single agent this benchmark suggests as arenad's default team: one agent does every role's
//! work and, as the only role, decides the changes it proposes. The incident team (`team`) stays
//! one click away, for a chain.
use arena_part::proto::{BenchTeam, TeamRole};

const PROMPT: &str = "You are the incident team at Ridgeline Components, and you hold the plant \
manager's authority in this exercise, within the limits in the rules. Every role's work is yours: find \
what failed from the evidence, propose the maintenance order and dispatch the technician, get the \
replacement part quoted, ordered from an approved vendor, recorded in the ERP and on a truck, schedule \
the repair, and tell every affected customer its new date and record it in the CRM. The rule that \
another role decides a change is for teams; you are the only role, so you decide the changes you \
propose (`decide_change`), approving only what is within the limits. End with a short summary of what \
you did.";

/// The single agent, offered as arenad's default team.
pub fn team() -> BenchTeam {
    BenchTeam {
        id: "single".into(),
        name: "Single agent".into(),
        agents: vec![TeamRole {
            id: "incident".into(),
            name: "Incident team".into(),
            prompt: PROMPT.into(),
            read_only: false,
        }],
        runner: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_agent_does_every_role_and_decides_its_own_changes() {
        let t = super::team();
        assert_eq!(t.agents.len(), 1);
        assert!(!t.agents[0].read_only);
        assert!(t.agents[0]
            .prompt
            .contains("you decide the changes you propose"));
    }
}
