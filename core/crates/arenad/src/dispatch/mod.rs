//! The `dispatch` runner: a lead and its team, generalised from the factory showcase's
//! hierarchical design. The lead (the team's agent marked `lead`, else its first) starts with the
//! ticket; every other agent starts idle. The lead gives work with `delegate`, agents report back
//! with `report` (or `message`), and an agent wakes, with its mail, whenever work or a message
//! arrives while it is idle. Agents stay alive across turns ([`crate::agents::session`]) and work
//! in parallel; arenad serialises their world steps behind the branch gate as in every runner.
//! The lead alone ends the run with `finish`; guard rails end it otherwise ([`board::Limits`]).
//! Delegations, reports, wakes, idles and the finish are recorded as events.

mod board;
mod inputs;
mod resume;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde_json::json;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

pub use board::Limits;
use board::{Action, Board, End, Input, Mail, Reason};

use crate::{
    agents::{
        session::{Session, SessionStart},
        AgentResult,
    },
    app::AppState,
    exec::{BranchCtx, Said},
    model::*,
    runner::{self, Brief, Ending},
};

/// How often the guard rails are looked at.
const TICK: Duration = Duration::from_secs(1);
/// How long agents still working when the lead finishes get to end their turn (every call they
/// make is refused by then).
const FINISH_GRACE: Duration = Duration::from_secs(45);
/// How long a session gets to end once told to.
const END_WAIT: Duration = Duration::from_secs(20);

impl Limits {
    /// The limits `ARENA_DISPATCH_MAX_MINUTES`, `ARENA_DISPATCH_IDLE_SECS` and
    /// `ARENA_DISPATCH_IDLE_WAKES` set, else the defaults (60 min, 20 s, 3).
    pub fn from_env() -> Limits {
        let num = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        let d = Limits::default();
        Limits {
            max_run: num("ARENA_DISPATCH_MAX_MINUTES")
                .map_or(d.max_run, |m| Duration::from_secs(m * 60)),
            idle_grace: num("ARENA_DISPATCH_IDLE_SECS").map_or(d.idle_grace, Duration::from_secs),
            max_idle_wakes: num("ARENA_DISPATCH_IDLE_WAKES").map_or(d.max_idle_wakes, |n| n as u32),
        }
    }
}

/// Runs a dispatch branch to its end: `None` when the lead finished it (the caller freezes and
/// scores the world), else how it ended (a failed run is scored here).
pub(crate) async fn run(
    state: &Arc<AppState>,
    ctx: &Arc<BranchCtx>,
    brief: &Brief,
) -> Result<Option<Ending>> {
    let events = state.store.events(&ctx.branch_id).await?;
    let mut crew = Crew::new(state, ctx, brief);
    let first = crew.begin(&events).await?;
    let end = crew.drive(first.into()).await?;
    crew.wind_down(&end).await?;
    Ok(match end {
        End::Finished => None,
        End::Failed(why) => {
            runner::freeze(ctx).await;
            runner::check(state, ctx).await?;
            Some((BranchStatus::Failed, Some(why)))
        }
        End::Broken(why) => Some((BranchStatus::InfraError, Some(why))),
        End::Stopped => Some(runner::STOPPED_ENDING),
    })
}

type Channel<T> = (mpsc::UnboundedSender<T>, mpsc::UnboundedReceiver<T>);

/// One agent's session, running on its own task.
struct Slot {
    inputs: mpsc::UnboundedSender<String>,
    task: JoinHandle<()>,
}

/// The dispatch runner's side of a branch: the board, the agents' sessions, and the channels
/// their turns and ends come back on.
struct Crew {
    state: Arc<AppState>,
    ctx: Arc<BranchCtx>,
    brief: Brief,
    board: Board,
    limits: Limits,
    slots: HashMap<String, Slot>,
    working: HashSet<String>,
    history: HashMap<String, String>,
    said: mpsc::UnboundedReceiver<Said>,
    turns: Channel<Input>,
    ends: Channel<(String, AgentResult)>,
    stop: watch::Sender<bool>,
    paused: bool,
    began: Instant,
}

impl Drop for Crew {
    /// Stopping the branch drops the runner where it is: its sessions go with it.
    fn drop(&mut self) {
        for slot in self.slots.values() {
            slot.task.abort();
        }
    }
}

impl Crew {
    fn new(state: &Arc<AppState>, ctx: &Arc<BranchCtx>, brief: &Brief) -> Crew {
        let ids: Vec<String> = ctx.team.agents.iter().map(|a| a.id.clone()).collect();
        let limits = Limits::from_env();
        Crew {
            state: state.clone(),
            ctx: ctx.clone(),
            brief: brief.clone(),
            board: Board::new(&ids, ctx.team.lead_index(), limits),
            limits,
            slots: HashMap::new(),
            working: HashSet::new(),
            history: HashMap::new(),
            said: ctx.listen(),
            turns: mpsc::unbounded_channel(),
            ends: mpsc::unbounded_channel(),
            stop: watch::channel(false).0,
            paused: false,
            began: Instant::now(),
        }
    }

    async fn record(
        &self,
        agent: &str,
        kind: EventKind,
        payload: serde_json::Value,
    ) -> Result<Event> {
        self.ctx
            .append(&self.state.store, Some(agent), kind, payload)
            .await
    }

    /// A new run starts the lead; a fork (its inherited record has agent starts) restarts every
    /// agent from it ([`resume`]). The starts are recorded first.
    async fn begin(&mut self, events: &[Event]) -> Result<Vec<Action>> {
        let Some(r) = resume::resumed(events, &self.ctx.team) else {
            let all: Vec<String> = self.ctx.team.agents.iter().map(|a| a.id.clone()).collect();
            self.record_starts(&all).await?;
            return Ok(self.board.start());
        };
        self.record_starts(&r.restart).await?;
        if !r.woke {
            return Ok(self.board.start());
        }
        self.history = r.history;
        Ok(self.board.resume(r.idle_wakes, r.pending, &r.working))
    }

    /// Records the start of each of `agents` (idle until it is woken).
    async fn record_starts(&self, agents: &[String]) -> Result<()> {
        for spec in self
            .ctx
            .team
            .agents
            .iter()
            .filter(|a| agents.contains(&a.id))
        {
            let mut start = json!({"name": spec.name, "kind": spec.kind, "model": spec.model});
            if let Some(p) = &spec.provider {
                start["provider"] = json!(p);
            }
            self.record(&spec.id, EventKind::AgentStart, start).await?;
        }
        Ok(())
    }

    /// Acts on what the board says and feeds it what happens, until the run ends.
    async fn drive(&mut self, mut queue: VecDeque<Action>) -> Result<End> {
        let mut tick = tokio::time::interval(TICK);
        loop {
            while let Some(action) = queue.pop_front() {
                match action {
                    Action::End(end) => return Ok(end),
                    Action::Wake {
                        agent,
                        reason,
                        mail,
                    } => self.wake(&agent, reason, &mail).await?,
                    Action::PassOn { agent, text } => {
                        queue.extend(self.pass_on(&agent, text).await?)
                    }
                }
            }
            let input = self.next_input(&mut tick).await?;
            queue.extend(self.board.step(input, self.began.elapsed()));
        }
    }

    /// The next thing that happens: a pause or resume, a stop, something an agent said, a turn
    /// that ended (recorded as the agent going idle), or time passing.
    async fn next_input(&mut self, tick: &mut tokio::time::Interval) -> Result<Input> {
        if self.ctx.flow.get() == crate::flow::Flow::Stopped {
            return Ok(Input::Stop);
        }
        if self.ctx.flow.is_paused() != self.paused {
            self.paused = !self.paused;
            return Ok(if self.paused {
                Input::Pause
            } else {
                Input::Resume
            });
        }
        tokio::select! {
            Some(said) = self.said.recv() => Ok(Input::Said(said)),
            Some(turn) = self.turns.1.recv() => {
                self.went_idle(&turn).await?;
                Ok(turn)
            }
            _ = tick.tick() => Ok(Input::Tick),
        }
    }

    async fn went_idle(&mut self, turn: &Input) -> Result<()> {
        let Input::TurnEnded {
            agent,
            ok,
            error,
            text,
            ..
        } = turn
        else {
            return Ok(());
        };
        self.working.remove(agent);
        let mut idle = json!({"ok": ok, "text": text});
        if let Some(e) = error {
            idle["error"] = json!(e);
        }
        self.record(agent, EventKind::Idle, idle).await.map(drop)
    }

    /// Records what `agent` said at the end of a turn as its report to the lead, and gives the
    /// board the report.
    async fn pass_on(&mut self, agent: &str, text: String) -> Result<Vec<Action>> {
        let lead = self.board.lead_id().to_string();
        let p = ReportPayload {
            to: lead.clone(),
            text: text.clone(),
            turn_end: true,
        };
        let e = self
            .record(agent, EventKind::Report, serde_json::to_value(&p)?)
            .await?;
        let mut said = Said::new(agent, EventKind::Report, Some(&lead), &text);
        said.seq = e.seq;
        Ok(self.board.step(Input::Said(said), self.began.elapsed()))
    }

    /// The input of `agent`'s turn: on its first, the brief, how the run works and (on a fork) its
    /// history; then why it was woken.
    fn input_for(&self, agent: &str, reason: Reason, mail: &[Mail]) -> String {
        let team = &self.ctx.team;
        let lead = self.board.lead_id();
        let mut input = String::new();
        if !self.slots.contains_key(agent) {
            input.push_str(&inputs::brief_text(&self.brief));
            input.push_str(&match agent == lead {
                true => inputs::how_for_lead(team, lead),
                false => inputs::how_for_member(team, lead),
            });
            if let Some(h) = self.history.get(agent) {
                input.push_str(&inputs::history_text(h));
            }
        }
        let max = self.limits.max_idle_wakes;
        input.push_str(&match reason {
            Reason::Start => String::new(),
            Reason::Resume => inputs::CARRY_ON.to_string(),
            Reason::Mail => inputs::mail_text(team, mail),
            Reason::Idle(n) => inputs::idle_text(team, lead, n, max, &self.board.statuses()),
        });
        input
    }

    /// Records `agent`'s wake and starts its turn (its session first, on its first turn).
    async fn wake(&mut self, agent: &str, reason: Reason, mail: &[Mail]) -> Result<()> {
        let input = self.input_for(agent, reason, mail);
        self.record(agent, EventKind::Wake, wake_payload(reason, mail, &input))
            .await?;
        self.working.insert(agent.to_string());
        if !self.slots.contains_key(agent) {
            if let Err(e) = self.open(agent) {
                let failed = Input::TurnEnded {
                    agent: agent.into(),
                    ok: false,
                    error: Some(e),
                    failure: None,
                    text: String::new(),
                };
                let _ = self.turns.0.send(failed);
                return Ok(());
            }
        }
        let _ = self.slots[agent].inputs.send(input);
        Ok(())
    }

    /// Starts `agent`'s session on its own task.
    fn open(&mut self, agent: &str) -> Result<(), String> {
        let spec = self
            .ctx
            .agent(agent)
            .ok_or_else(|| format!("no agent {agent}"))?;
        let driver = runner::driver_for(&self.state, spec.kind).map_err(|e| format!("{e:#}"))?;
        let start = SessionStart {
            branch: self.ctx.clone(),
            spec,
            store: self.state.store.clone(),
        };
        let session = driver.session(start)?;
        let (inputs, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(serve(
            session,
            agent.to_string(),
            rx,
            self.turns.0.clone(),
            self.ends.0.clone(),
            self.stop.subscribe(),
        ));
        self.slots.insert(agent.to_string(), Slot { inputs, task });
        Ok(())
    }

    /// Ends the run's agents: nothing more may be done in the world; when the lead finished,
    /// agents still working get a while to end their turn; then every session ends and every
    /// agent's end is recorded.
    async fn wind_down(&mut self, end: &End) -> Result<()> {
        self.ctx.close();
        if *end == End::Finished {
            self.let_turns_end().await?;
        }
        let _ = self.stop.send(true);
        let results = self.collect_ends().await;
        for spec in self.ctx.team.agents.clone() {
            let payload = match results.get(&spec.id) {
                Some(r) => runner::agent_end(r),
                None if self.slots.contains_key(&spec.id) => runner::agent_end(&lost(&spec.id)),
                None => never_worked(),
            };
            self.record(
                &spec.id,
                EventKind::AgentEnd,
                serde_json::to_value(payload)?,
            )
            .await?;
        }
        Ok(())
    }

    async fn let_turns_end(&mut self) -> Result<()> {
        let until = tokio::time::Instant::now() + FINISH_GRACE;
        while !self.working.is_empty() {
            let next = tokio::time::timeout_at(until, self.turns.1.recv()).await;
            match next {
                Ok(Some(turn)) => self.went_idle(&turn).await?,
                _ => break,
            }
        }
        Ok(())
    }

    async fn collect_ends(&mut self) -> HashMap<String, AgentResult> {
        let mut got = HashMap::new();
        let until = tokio::time::Instant::now() + END_WAIT;
        while got.len() < self.slots.len() {
            match tokio::time::timeout_at(until, self.ends.1.recv()).await {
                Ok(Some((agent, r))) => {
                    got.insert(agent, r);
                }
                _ => break,
            }
        }
        got
    }
}

/// A wake's payload: `{reason, mail, idle_wake?, input}` (`mail`: the seqs of the recorded
/// mail it delivers; arena's own notices are in the input only).
fn wake_payload(reason: Reason, mail: &[Mail], input: &str) -> serde_json::Value {
    let (word, n) = match reason {
        Reason::Start => ("start", None),
        Reason::Mail => ("mail", None),
        Reason::Idle(n) => ("idle", Some(n)),
        Reason::Resume => ("resume", None),
    };
    let recorded = mail.iter().filter(|m| m.kind != board::MailKind::Notice);
    let seqs: Vec<i64> = recorded.map(|m| m.seq).collect();
    let mut wake = json!({"reason": word, "mail": seqs, "input": input});
    if let Some(n) = n {
        wake["idle_wake"] = json!(n);
    }
    wake
}

/// An agent's end when its session never said how it ended.
fn lost(agent: &str) -> AgentResult {
    AgentResult {
        ok: false,
        error: Some(format!(
            "{agent}'s session did not end in time when the run ended"
        )),
        tokens: crate::agents::tokens_json(0, 0, 0),
        cost_usd: None,
        cost_source: None,
        seconds: 0.0,
        failure: None,
    }
}

/// The end of an agent that was never given work: it used nothing.
fn never_worked() -> AgentEndPayload {
    AgentEndPayload {
        ok: true,
        error: None,
        tokens: Some(0),
        prompt_tokens: Some(0),
        completion_tokens: Some(0),
        cost_usd: Some(0.0),
        cost_source: None,
        seconds: Some(0.0),
    }
}

/// Runs `agent`'s session: a turn for each input, each turn's end passed back, until the run
/// tells it to stop (a turn still going is cut off); then ends the session and passes back its
/// totals.
async fn serve(
    mut session: Box<dyn Session>,
    agent: String,
    mut inputs: mpsc::UnboundedReceiver<String>,
    turns: mpsc::UnboundedSender<Input>,
    ends: mpsc::UnboundedSender<(String, AgentResult)>,
    mut stop: watch::Receiver<bool>,
) {
    let mut cut = false;
    loop {
        let input = tokio::select! {
            i = inputs.recv() => i,
            _ = stop.wait_for(|s| *s) => None,
        };
        let Some(input) = input else { break };
        let turn = tokio::select! {
            t = session.turn(input) => Some(t),
            _ = stop.wait_for(|s| *s) => None,
        };
        let Some(t) = turn else {
            cut = true;
            break;
        };
        let ended = Input::TurnEnded {
            agent: agent.clone(),
            ok: t.ok,
            error: t.error,
            failure: t.failure,
            text: t.text,
        };
        let _ = turns.send(ended);
    }
    let result = session.end(cut).await;
    let _ = ends.send((agent, result));
}
