//! What arenad gives agents where the benchmark has no say: the team it suggests and the heading
//! over the benchmark's instructions. A benchmark's own `team` and `instructions_title` win.
use parts::proto::{BenchDescribe, BenchTeam, TeamRole};

use crate::model::{AgentKind, AgentSpec, Team};

/// The heading over the instructions when the benchmark gives none.
pub const INSTRUCTIONS_TITLE: &str = "Instructions";

/// The prompt of the one agent arenad suggests when the benchmark gives no team.
pub const GENERIC_PROMPT: &str = "You are an agent working on the task in your input. \
Follow the instructions in your input and complete the task by calling the tools you have been given. \
Check what you need with read tools before changing anything. When done, end with a short summary of \
what you did.";

/// The model of the suggested agents.
const MODEL: &str = "sonnet";

/// The heading over `bench`'s instructions in each agent's input.
pub fn instructions_title(bench: &BenchDescribe) -> &str {
    bench
        .instructions_title
        .as_deref()
        .unwrap_or(INSTRUCTIONS_TITLE)
}

/// The team suggested for `bench`: its own, or one generic agent. Every agent is Claude Code.
pub fn default_team(bench: &BenchDescribe) -> Team {
    bench.team.as_ref().map_or_else(generic_team, team_of)
}

fn generic_team() -> Team {
    let solo = TeamRole {
        id: "agent".into(),
        name: "Agent".into(),
        prompt: GENERIC_PROMPT.into(),
    };
    Team {
        id: "single-agent".into(),
        name: "Single agent".into(),
        agents: vec![agent(&solo)],
    }
}

fn team_of(t: &BenchTeam) -> Team {
    Team {
        id: t.id.clone(),
        name: t.name.clone(),
        agents: t.agents.iter().map(agent).collect(),
    }
}

fn agent(role: &TeamRole) -> AgentSpec {
    AgentSpec {
        id: role.id.clone(),
        name: role.name.clone(),
        kind: AgentKind::ClaudeCode,
        model: Some(MODEL.into()),
        prompt: role.prompt.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bench(team: Option<BenchTeam>, title: Option<&str>) -> BenchDescribe {
        BenchDescribe {
            name: "b".into(),
            version: "0.1.0".into(),
            protocol: 1,
            world: vec![],
            tasks: vec![],
            instructions: "Do it well.".into(),
            instructions_title: title.map(Into::into),
            team,
            tools: vec![],
        }
    }

    #[test]
    fn a_benchmark_without_a_team_gets_one_generic_agent() {
        let t = default_team(&bench(None, None));
        assert_eq!(t.agents.len(), 1);
        let a = &t.agents[0];
        assert_eq!(
            (a.kind, a.model.as_deref()),
            (AgentKind::ClaudeCode, Some("sonnet"))
        );
        assert_eq!(a.prompt, GENERIC_PROMPT);
        let lower = GENERIC_PROMPT.to_lowercase();
        for word in ["retail", "customer", "store", "order", "ticket", "policy"] {
            assert!(!lower.contains(word), "{word} in the generic prompt");
        }
    }

    #[test]
    fn a_benchmark_team_is_used_as_given() {
        let role = |id: &str| TeamRole {
            id: id.into(),
            name: id.to_uppercase(),
            prompt: format!("be {id}"),
        };
        let given = BenchTeam {
            id: "pair".into(),
            name: "Pair".into(),
            agents: vec![role("first"), role("second")],
        };
        let t = default_team(&bench(Some(given), None));
        assert_eq!((t.id.as_str(), t.name.as_str()), ("pair", "Pair"));
        let got: Vec<_> = t
            .agents
            .iter()
            .map(|a| (&*a.id, &*a.name, &*a.prompt))
            .collect();
        assert_eq!(
            got,
            [
                ("first", "FIRST", "be first"),
                ("second", "SECOND", "be second")
            ]
        );
        assert!(t.agents.iter().all(|a| a.kind == AgentKind::ClaudeCode));
    }

    #[test]
    fn the_instructions_title_is_the_benchmarks_or_generic() {
        assert_eq!(instructions_title(&bench(None, None)), "Instructions");
        assert_eq!(instructions_title(&bench(None, Some("Rules"))), "Rules");
    }
}
