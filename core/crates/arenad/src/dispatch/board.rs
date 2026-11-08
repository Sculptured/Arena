//! The dispatch runner's state machine, with no agent and no clock of its own: what each agent is
//! doing (idle, working, failed), the mail waiting for it, and the guard rails. It takes what
//! happened (an agent said something, a turn ended, time passed, a pause) and answers what to do
//! (wake an agent with its mail, pass on what an agent said at the end of a turn, end the run).

use std::time::Duration;

use crate::{agents::FailureKind, exec::Said, model::EventKind};

/// The run's guard rails.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// The longest the run may go on, time paused left out.
    pub max_run: Duration,
    /// How long the whole team may wait, with no mail, before the lead is woken.
    pub idle_grace: Duration,
    /// How many times the lead is woken for an idle team before the run ends as failed.
    pub max_idle_wakes: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_run: Duration::from_secs(60 * 60),
            idle_grace: Duration::from_secs(20),
            max_idle_wakes: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Idle,
    Working,
    Failed(String),
}

/// What kind of mail an agent gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailKind {
    Delegation,
    Report,
    Message,
    /// arena's own notice to the lead: an agent failed, or work for it could not be delivered.
    Notice,
}

/// One piece of mail: what was said (at step `seq`), by whom, and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Mail {
    pub seq: i64,
    pub from: String,
    pub kind: MailKind,
    pub text: String,
}

/// Why an agent is woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The lead's first turn, with the ticket.
    Start,
    /// Mail arrived.
    Mail,
    /// The whole team waited and the lead has not finished: the lead's `n`th such wake.
    Idle(u32),
    /// A fork: the agent was working at the fork point and carries on.
    Resume,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Said(Said),
    TurnEnded {
        agent: String,
        ok: bool,
        error: Option<String>,
        failure: Option<FailureKind>,
        text: String,
    },
    /// Time passed (the board is told the time with every input).
    Tick,
    Pause,
    Resume,
    Stop,
}

/// How a run ends.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// The lead finished it: score the world.
    Finished,
    /// A guard rail or the lead's failure ended it: scored, and failed with this reason.
    Failed(String),
    /// The lead's agent broke (not the model's doing): an infrastructure error.
    Broken(String),
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start a turn of `agent` with `mail`.
    Wake {
        agent: String,
        reason: Reason,
        mail: Vec<Mail>,
    },
    /// `agent` ended a turn without reporting: pass on what it said to the lead as its report
    /// (record it, then hand the board the recorded `Said`).
    PassOn {
        agent: String,
        text: String,
    },
    End(End),
}

struct Agent {
    id: String,
    state: State,
    mail: Vec<Mail>,
    /// It told the lead something during its current turn.
    reported: bool,
    /// What it said at the end of its last turn.
    last: String,
}

pub struct Board {
    agents: Vec<Agent>,
    lead: usize,
    limits: Limits,
    paused_since: Option<Duration>,
    paused_total: Duration,
    idle_since: Option<Duration>,
    idle_wakes: u32,
    ended: bool,
}

/// What an agent is doing and what it said last, for the lead's idle wake.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub id: String,
    pub state: State,
    pub last: String,
}

impl Board {
    /// The board of a team of `ids` led by `ids[lead]`, everyone idle.
    pub fn new(ids: &[String], lead: usize, limits: Limits) -> Board {
        let agent = |id: &String| Agent {
            id: id.clone(),
            state: State::Idle,
            mail: vec![],
            reported: false,
            last: String::new(),
        };
        Board {
            agents: ids.iter().map(agent).collect(),
            lead,
            limits,
            paused_since: None,
            paused_total: Duration::ZERO,
            idle_since: None,
            idle_wakes: 0,
            ended: false,
        }
    }

    /// A new run: the lead's first turn.
    pub fn start(&mut self) -> Vec<Action> {
        let lead = self.agents[self.lead].id.clone();
        vec![self.wake(&lead, Reason::Start, vec![])]
    }

    /// A fork: the lead's idle wakes so far, each agent's mail not yet delivered, and the agents
    /// that were working at the fork point carry on (the others wait for their mail).
    pub fn resume(
        &mut self,
        idle_wakes: u32,
        pending: Vec<(String, Mail)>,
        working: &[String],
    ) -> Vec<Action> {
        self.idle_wakes = idle_wakes;
        for (to, mail) in pending {
            if let Some(a) = self.find(&to) {
                a.mail.push(mail);
            }
        }
        let mut out: Vec<Action> = working
            .iter()
            .map(|id| self.wake(id, Reason::Resume, vec![]))
            .collect();
        out.extend(self.deliver_all());
        out
    }

    pub fn statuses(&self) -> Vec<Status> {
        let status = |a: &Agent| Status {
            id: a.id.clone(),
            state: a.state.clone(),
            last: a.last.clone(),
        };
        self.agents.iter().map(status).collect()
    }

    pub fn lead_id(&self) -> &str {
        &self.agents[self.lead].id
    }

    /// The run's time so far, time paused left out, at `now` (time since the run began).
    fn active(&self, now: Duration) -> Duration {
        let open = self
            .paused_since
            .map_or(Duration::ZERO, |p| now.saturating_sub(p));
        now.saturating_sub(self.paused_total + open)
    }

    fn find(&mut self, id: &str) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Takes what happened at `now` (time since the run began); what to do about it.
    pub fn step(&mut self, input: Input, now: Duration) -> Vec<Action> {
        if self.ended {
            return vec![];
        }
        let out = match input {
            Input::Said(s) => self.said(s),
            Input::TurnEnded {
                agent,
                ok,
                error,
                failure,
                text,
            } => match ok {
                true => self.turn_done(&agent, text),
                false => self.turn_failed(&agent, error.unwrap_or_default(), failure),
            },
            Input::Tick => self.tick(now),
            Input::Pause => self.pause(now),
            Input::Resume => self.unpause(now),
            Input::Stop => vec![Action::End(End::Stopped)],
        };
        if out.iter().any(|a| matches!(a, Action::End(_))) {
            self.ended = true;
        }
        out
    }

    fn pause(&mut self, now: Duration) -> Vec<Action> {
        self.paused_since.get_or_insert(now);
        vec![]
    }

    /// The pause is over: the mail that waited for idle agents is delivered.
    fn unpause(&mut self, now: Duration) -> Vec<Action> {
        if let Some(p) = self.paused_since.take() {
            self.paused_total += now.saturating_sub(p);
        }
        self.idle_since = None;
        self.deliver_all()
    }

    fn deliver_all(&mut self) -> Vec<Action> {
        let ids: Vec<String> = self.agents.iter().map(|a| a.id.clone()).collect();
        ids.iter().filter_map(|id| self.deliver(id)).collect()
    }

    /// Wakes `id` with its mail when it is idle, has mail and the run isn't paused.
    fn deliver(&mut self, id: &str) -> Option<Action> {
        let paused = self.paused_since.is_some();
        let a = self.find(id)?;
        if paused || a.state != State::Idle || a.mail.is_empty() {
            return None;
        }
        let mail = std::mem::take(&mut a.mail);
        Some(self.wake(id, Reason::Mail, mail))
    }

    fn wake(&mut self, id: &str, reason: Reason, mail: Vec<Mail>) -> Action {
        if let Some(a) = self.find(id) {
            a.state = State::Working;
            a.reported = false;
        }
        self.idle_since = None;
        Action::Wake {
            agent: id.to_string(),
            reason,
            mail,
        }
    }

    fn said(&mut self, s: Said) -> Vec<Action> {
        if s.kind == EventKind::Finish {
            return vec![Action::End(End::Finished)];
        }
        let lead = self.lead_id().to_string();
        let to_lead = s.to.as_deref().is_some_and(|t| t == lead || t == "*");
        if to_lead {
            if let Some(a) = self.find(&s.from) {
                a.reported = true;
            }
        }
        let to = self.recipients(&s);
        let kind = match s.kind {
            EventKind::Delegation => MailKind::Delegation,
            EventKind::Report => MailKind::Report,
            _ => MailKind::Message,
        };
        let mail = Mail {
            seq: s.seq,
            from: s.from,
            kind,
            text: s.text,
        };
        to.iter()
            .flat_map(|id| self.post(id, mail.clone()))
            .collect()
    }

    /// Who `s` is for: its `to`, or every agent but its sender for `*`.
    fn recipients(&self, s: &Said) -> Vec<String> {
        match s.to.as_deref() {
            Some("*") => (self.agents.iter())
                .map(|a| a.id.clone())
                .filter(|id| *id != s.from)
                .collect(),
            Some(t) if t != s.from => vec![t.to_string()],
            _ => vec![],
        }
    }

    /// Puts `mail` in `id`'s box and wakes it if it can; work for a failed agent goes back to the
    /// lead as a notice.
    fn post(&mut self, id: &str, mail: Mail) -> Vec<Action> {
        let Some(a) = self.find(id) else {
            return vec![];
        };
        if let State::Failed(why) = &a.state {
            if mail.kind != MailKind::Delegation {
                return vec![];
            }
            let text = format!("{id} has stopped working ({why}), so your delegation at step {} was not delivered.", mail.seq);
            return self.notify_lead(mail.seq, text);
        }
        a.mail.push(mail);
        self.deliver(id).into_iter().collect()
    }

    fn notify_lead(&mut self, seq: i64, text: String) -> Vec<Action> {
        let lead = self.lead_id().to_string();
        let notice = Mail {
            seq,
            from: "arena".into(),
            kind: MailKind::Notice,
            text,
        };
        self.find(&lead)
            .expect("the lead is on the board")
            .mail
            .push(notice);
        self.deliver(&lead).into_iter().collect()
    }

    fn turn_done(&mut self, id: &str, text: String) -> Vec<Action> {
        let is_lead = id == self.lead_id();
        let Some(a) = self.find(id) else {
            return vec![];
        };
        a.state = State::Idle;
        a.last = text.clone();
        let pass_on = !is_lead && !a.reported && !text.trim().is_empty();
        let mut out = vec![];
        if pass_on {
            out.push(Action::PassOn {
                agent: id.to_string(),
                text,
            });
        }
        out.extend(self.deliver(id));
        out
    }

    fn turn_failed(
        &mut self,
        id: &str,
        error: String,
        failure: Option<FailureKind>,
    ) -> Vec<Action> {
        if id == self.lead_id() {
            let why = format!("the lead failed: {error}");
            let end = match failure {
                Some(FailureKind::Model) => End::Failed(why),
                None => End::Broken(why),
            };
            return vec![Action::End(end)];
        }
        let Some(a) = self.find(id) else {
            return vec![];
        };
        a.state = State::Failed(error.clone());
        a.mail.clear();
        self.notify_lead(
            0,
            format!("{id} stopped working: {error}. It can't take more work."),
        )
    }

    fn tick(&mut self, now: Duration) -> Vec<Action> {
        if self.paused_since.is_some() {
            return vec![];
        }
        let active = self.active(now);
        if active >= self.limits.max_run {
            let mins = self.limits.max_run.as_secs_f64() / 60.0;
            let why = format!(
                "the run reached its time limit ({mins:.0} min) before the lead finished it"
            );
            return vec![Action::End(End::Failed(why))];
        }
        let waiting = |a: &Agent| a.state != State::Working && a.mail.is_empty();
        if !self.agents.iter().all(waiting) {
            self.idle_since = None;
            return vec![];
        }
        let since = *self.idle_since.get_or_insert(active);
        if active.saturating_sub(since) < self.limits.idle_grace {
            return vec![];
        }
        self.idle_wake()
    }

    /// The team waited out the grace: the lead is woken, or after too many wakes the run fails.
    fn idle_wake(&mut self) -> Vec<Action> {
        if self.idle_wakes >= self.limits.max_idle_wakes {
            let n = self.limits.max_idle_wakes;
            let why = format!("the team stayed idle after the lead was woken {n} times, and the lead never finished the run");
            return vec![Action::End(End::Failed(why))];
        }
        self.idle_wakes += 1;
        let lead = self.lead_id().to_string();
        if let Some(a) = self.find(&lead) {
            a.state = State::Idle;
        }
        vec![self.wake(&lead, Reason::Idle(self.idle_wakes), vec![])]
    }
}

#[cfg(test)]
mod tests;
