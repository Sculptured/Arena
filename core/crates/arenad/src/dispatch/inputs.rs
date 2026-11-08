//! What the dispatch runner tells an agent when it wakes it: on its first turn the task's ticket,
//! the benchmark's instructions and how a dispatch run works (with its history, on a fork); then
//! the mail that woke it, or the state of the team when the lead is woken for an idle team.

use super::board::{Mail, MailKind, State, Status};
use crate::{model::Team, runner::Brief};

/// The ticket and the instructions under their title.
pub fn brief_text(b: &Brief) -> String {
    format!("{}\n# {}\n\n{}\n", b.ticket, b.title, b.instructions)
}

/// `id`'s name in `team` with its id: "Quality assurance (`quality`)".
pub fn named(team: &Team, id: &str) -> String {
    match team.agents.iter().find(|a| a.id == id) {
        Some(a) if a.name != a.id => format!("{} (`{id}`)", a.name),
        _ => format!("`{id}`"),
    }
}

/// How a dispatch run works, for the lead.
pub fn how_for_lead(team: &Team, lead: &str) -> String {
    let others: Vec<String> = (team.agents.iter())
        .filter(|a| a.id != lead)
        .map(|a| format!("- {}", named(team, &a.id)))
        .collect();
    format!(
        "\n# How this run works\n\nYou lead this run. Your team:\n{}\n\nGive each agent its work with \
         `delegate {{to, task}}`, with the facts and ids it needs: they work in parallel, report back \
         to you, and stay available for more. You are woken when a report or a message arrives. \
         Check what they report, follow up, and when everything is done and checked call \
         `finish {{summary}}`: only you can end the run, and nothing can be done after it.\n",
        others.join("\n")
    )
}

/// How a dispatch run works, for an agent the lead gives work.
pub fn how_for_member(team: &Team, lead: &str) -> String {
    format!(
        "\n# How this run works\n\n{} leads this run and gives you work. Do your part, then \
         `report {{text}}` to the lead with the facts and ids it needs. You then wait: you are woken \
         when more work or a message arrives. You may `message` another agent of the team directly.\n",
        named(team, lead)
    )
}

fn heading(team: &Team, m: &Mail) -> String {
    let from = named(team, &m.from);
    match m.kind {
        MailKind::Delegation => format!("Work from {from}, step {}", m.seq),
        MailKind::Report => format!("Report from {from}, step {}", m.seq),
        MailKind::Message => format!("Message from {from}, step {}", m.seq),
        MailKind::Notice => "Notice from arena".to_string(),
    }
}

/// The mail that woke an agent, oldest first.
pub fn mail_text(team: &Team, mail: &[Mail]) -> String {
    let each = mail
        .iter()
        .map(|m| format!("\n# {}\n\n{}\n", heading(team, m), m.text));
    each.collect()
}

/// The longest part of what an agent said last that the lead's idle wake quotes.
const QUOTE: usize = 400;

fn quoted(text: &str) -> String {
    let t = text.trim();
    match t.char_indices().nth(QUOTE) {
        Some((i, _)) => format!("{}…", &t[..i]),
        None => t.to_string(),
    }
}

fn status_line(team: &Team, s: &Status) -> String {
    let who = named(team, &s.id);
    match &s.state {
        State::Failed(why) => format!("- {who}: stopped working ({why})"),
        _ if s.last.trim().is_empty() => format!("- {who}: waiting, has not worked yet"),
        _ => format!("- {who}: waiting; said last: {}", quoted(&s.last)),
    }
}

/// The lead's `n`th wake (of `max`) for a team that waits with nothing to do.
pub fn idle_text(team: &Team, lead: &str, n: u32, max: u32, statuses: &[Status]) -> String {
    let lines: Vec<String> = (statuses.iter())
        .filter(|s| s.id != lead)
        .map(|s| status_line(team, s))
        .collect();
    format!(
        "\n# The team is waiting\n\nEvery agent is waiting and you have not finished the run (wake {n} \
         of {max}; after that the run ends as failed).\n{}\n\nDelegate what remains, follow up on \
         what is missing, or call `finish {{summary}}` if the work is done and checked.\n",
        lines.join("\n")
    )
}

/// The start of an agent's first turn after a fork: what it did and got before the fork point.
pub fn history_text(history: &str) -> String {
    format!(
        "\n# Your work so far\n\nThis run was forked: you are starting again from a recorded point. \
         Here is what you did and were told before it, oldest first; the world is as it was then.\n\n{history}\n"
    )
}

/// What an agent that was working at the fork point is told.
pub const CARRY_ON: &str = "\n# Carry on\n\nCarry on with your work from where it stood.\n";
