//! The team this benchmark suggests (triage, then resolver) and the heading over its policy. Triage
//! is read-only, as every team was before arena let any agent write, so runs stay comparable.
use arena_part::proto::{BenchTeam, TeamRole};

/// The heading over the store policy in each agent's input.
pub const INSTRUCTIONS_TITLE: &str = "Store policy";

pub const TRIAGE_PROMPT: &str = "You are the triage agent of a retail customer-service team. \
Read the customer ticket and the store policy in your input.
Using only the read tools, identify the customer (user_id), the orders and items involved, and exactly what
the customer wants, including any conditions they gave. Do not change anything: writes are refused for you.
Then call the handoff tool with your note for the resolver agent: the user_id, the relevant order ids, item ids and payment
methods you found, and the customer's request and conditions in full. Keep it factual.
Use only the tools you have been given; do not edit the database in any other way.";

pub const RESOLVER_PROMPT: &str = "You are the resolver agent of a retail customer-service team. \
Your input holds the handoff note from the
triage agent, the customer ticket and the store policy. Resolve the customer's request completely by calling the tools,
following the policy. The customer has already confirmed every action described in the ticket, so do not
wait for confirmation. Verify details with read tools before writing. When done, end with a short summary
listing each change you made.
Use only the tools you have been given; do not edit the database in any other way.";

fn role(id: &str, name: &str, prompt: &str, read_only: bool) -> TeamRole {
    TeamRole {
        tools: Vec::new(),
        id: id.into(),
        name: name.into(),
        prompt: prompt.into(),
        read_only,
    }
}

pub fn team() -> BenchTeam {
    BenchTeam {
        id: "triage-resolver".into(),
        name: "Triage and resolver".into(),
        agents: vec![
            role("triage", "Triage", TRIAGE_PROMPT, true),
            role("resolver", "Resolver", RESOLVER_PROMPT, false),
        ],
        runner: Some("chain".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_reads_and_the_resolver_writes() {
        let t = team();
        let read_only: Vec<_> = t.agents.iter().map(|a| (&*a.id, a.read_only)).collect();
        assert_eq!(read_only, [("triage", true), ("resolver", false)]);
    }
}
