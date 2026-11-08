//! One Claude Code agent as a long-lived headless process, for the dispatch runner: the same
//! isolation as the one-shot driver (`claude::session_command`), taking each turn as a user
//! message on stdin (stream-json) and ending it with the stream's `result`. Each model call the
//! stream shows is recorded as a `model_call` reported by the client (see `usage`).

use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::mpsc,
};

use super::{
    session::{end_error, BoxFuture, Session, SessionStart, Turn},
    tail,
    usage::UsageMeter,
    AgentResult, Mcp, CLI_TIMEOUT_SECS,
};
use crate::{exec::BranchCtx, external::UsageReport, model::EventKind, store::Store};

pub struct ClaudeSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::UnboundedReceiver<Value>,
    stderr: Arc<Mutex<String>>,
    /// The process has written something (it didn't die at start).
    spoke: bool,
    respawns: u32,
    mcp: Mcp,
    binary: String,
    spec: crate::model::AgentSpec,
    workdir: PathBuf,
    branch: Arc<BranchCtx>,
    agent: String,
    store: Arc<Store>,
    /// The agent's prompt, sent before the first turn's input.
    prompt: Option<String>,
    meter: UsageMeter,
    /// Claude Code's cost of the session so far (cumulative over turns).
    cost: Option<f64>,
    error: Option<String>,
    busy: Duration,
}

/// Starts `command` in `workdir` with its input, output and errors piped (killed on drop).
fn spawn(command: std::process::Command, workdir: &std::path::Path) -> Result<Process, String> {
    let mut cmd = tokio::process::Command::from(command);
    cmd.current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start claude: {e}"))?;
    let (tx, lines) = mpsc::unbounded_channel();
    read_lines(child.stdout.take().expect("stdout is piped"), tx);
    let stderr = keep_stderr(child.stderr.take().expect("stderr is piped"));
    Ok(Process {
        stdin: child.stdin.take(),
        child,
        lines,
        stderr,
    })
}

/// Reads stdout as JSON lines, passing each on; ends when the process closes it.
fn read_lines(stdout: tokio::process::ChildStdout, tx: mpsc::UnboundedSender<Value>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(v);
            }
        }
    });
}

/// Keeps what the process writes to stderr (its tail is reported when it fails).
fn keep_stderr(mut stderr: tokio::process::ChildStderr) -> Arc<Mutex<String>> {
    let kept = Arc::new(Mutex::new(String::new()));
    let into = kept.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let mut k = into.lock().unwrap();
            k.push_str(&String::from_utf8_lossy(&buf[..n]));
            let keep = tail(&k).to_string();
            *k = keep;
        }
    });
    kept
}

/// The process of one session: its input, its stdout as JSON values, and its stderr's tail.
struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::UnboundedReceiver<Value>,
    stderr: Arc<Mutex<String>>,
}

/// How many times a process that dies before it says anything is started again: Claude Code
/// (its Bun runtime) has crashed at start now and then when several start at once.
const RESPAWNS: u32 = 2;

impl ClaudeSession {
    /// Starts the process in a fresh work folder, its MCP endpoint the agent's on the branch.
    pub fn start(self_base: &str, binary: &str, s: SessionStart) -> Result<ClaudeSession, String> {
        let workdir = super::fresh_workdir().map_err(|e| format!("cannot create workdir: {e}"))?;
        let mcp = Mcp::for_agent(self_base, &s.branch, &s.spec.id);
        let command = super::claude::session_command(binary, &s.spec, &mcp);
        let p = spawn(command, &workdir).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&workdir);
        })?;
        Ok(ClaudeSession {
            stdin: p.stdin,
            child: p.child,
            lines: p.lines,
            stderr: p.stderr,
            spoke: false,
            respawns: 0,
            mcp,
            binary: binary.to_string(),
            spec: s.spec.clone(),
            workdir,
            branch: s.branch,
            agent: s.spec.id,
            store: s.store,
            prompt: Some(s.spec.prompt),
            meter: UsageMeter::default(),
            cost: None,
            error: None,
            busy: Duration::ZERO,
        })
    }

    /// Starts the process again (it died before it said anything) and resends `text`.
    async fn respawn(&mut self, text: &str) -> Result<(), String> {
        self.respawns += 1;
        eprintln!(
            "branch {}: {}'s claude died before it said anything; starting it again",
            self.branch.branch_id, self.agent
        );
        let command = super::claude::session_command(&self.binary, &self.spec, &self.mcp);
        let p = spawn(command, &self.workdir)?;
        (self.child, self.stdin, self.lines, self.stderr) = (p.child, p.stdin, p.lines, p.stderr);
        self.send(text).await
    }

    async fn send(&mut self, text: &str) -> Result<(), String> {
        let msg = json!({"type": "user", "message": {"role": "user", "content": text}});
        let stdin = self.stdin.as_mut().ok_or("claude's input is closed")?;
        let line = format!("{msg}\n");
        let wrote = stdin.write_all(line.as_bytes()).await;
        wrote
            .and(stdin.flush().await)
            .map_err(|e| format!("claude has exited: {e}{}", self.stderr_tail()))
    }

    fn stderr_tail(&self) -> String {
        let k = self.stderr.lock().unwrap();
        match k.trim().is_empty() {
            true => String::new(),
            false => format!("\nstderr: {}", k.trim()),
        }
    }

    async fn record(&self, report: &UsageReport) {
        let p = report.payload();
        let at = Some(self.agent.as_str());
        let appended = self.branch.append(&self.store, at, EventKind::ModelCall, p);
        if let Err(e) = appended.await {
            eprintln!(
                "branch {}: recording a model call: {e:#}",
                self.branch.branch_id
            );
        }
    }

    /// Records the model call an assistant message shows, or a compaction of the context.
    async fn on_line(&mut self, v: &Value) {
        let report = match v["type"].as_str() {
            Some("assistant") => self.meter.assistant(&v["message"]),
            Some("system") if v["subtype"] == "compact_boundary" => Some(UsageReport {
                compaction: Some(v["compact_metadata"].clone()),
                ..UsageReport::default()
            }),
            _ => None,
        };
        if let Some(r) = report {
            self.record(&r).await;
        }
    }

    /// Reads the stream until the turn's `result`; the turn it ends. `None` when the process died
    /// before it said anything, and may be started again.
    async fn until_result(&mut self) -> Option<Turn> {
        while let Some(v) = self.lines.recv().await {
            self.spoke = true;
            if v["type"] == "result" {
                return Some(self.ended_turn(&v).await);
            }
            self.on_line(&v).await;
        }
        if !self.spoke && self.respawns < RESPAWNS {
            return None;
        }
        let code = self.child.try_wait().ok().flatten();
        let how = code.map_or("".into(), |c| format!(" ({c})"));
        let why = format!(
            "claude exited{how} in the middle of the run{}",
            self.stderr_tail()
        );
        Some(Turn::failed(why, None))
    }

    /// Reads the turn to its end, starting the process again (and resending `text`) when it
    /// died before it said anything.
    async fn read_turn(&mut self, text: &str) -> Turn {
        loop {
            if let Some(turn) = self.until_result().await {
                return turn;
            }
            if let Err(e) = self.respawn(text).await {
                return Turn::failed(e, None);
            }
        }
    }

    async fn ended_turn(&mut self, result: &Value) -> Turn {
        for r in self.meter.turn_ended(result) {
            self.record(&r).await;
        }
        self.cost = result["total_cost_usd"].as_f64().or(self.cost);
        let text = result["result"].as_str().unwrap_or("").to_string();
        if !result["is_error"].as_bool().unwrap_or(false) {
            return Turn::done(text);
        }
        let subtype = result["subtype"].as_str().unwrap_or("error");
        let error = if text.is_empty() {
            format!("claude reported an error ({subtype})")
        } else {
            text
        };
        Turn::failed(error, None)
    }

    async fn run_turn(&mut self, input: String) -> Turn {
        let text = match self.prompt.take() {
            Some(p) => format!("{p}\n\n{input}"),
            None => input,
        };
        if let Err(e) = self.send(&text).await {
            return Turn::failed(e, None);
        }
        let since = Instant::now();
        let limit = Duration::from_secs(CLI_TIMEOUT_SECS);
        let branch = self.branch.clone();
        let turn = branch.flow.timeout(limit, self.read_turn(&text)).await;
        self.busy += since.elapsed();
        let turn = turn.unwrap_or_else(|| {
            Turn::failed(format!("a turn timed out after {CLI_TIMEOUT_SECS} s"), None)
        });
        if !turn.ok {
            self.error = turn.error.clone();
        }
        turn
    }

    async fn close(mut self, cut: bool) -> AgentResult {
        drop(self.stdin.take());
        let exited = tokio::time::timeout(Duration::from_secs(10), self.child.wait()).await;
        if exited.is_err() {
            let _ = self.child.kill().await;
        }
        let (p, c) = (self.meter.prompt_tokens, self.meter.completion_tokens);
        let error = end_error(self.error.take(), cut);
        AgentResult {
            ok: error.is_none(),
            error,
            tokens: super::tokens_json(p, c, p + c),
            cost_usd: self.cost,
            cost_source: self.cost.map(|_| "api_equivalent_estimate".into()),
            seconds: self.busy.as_secs_f64(),
            failure: None,
        }
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
    }
}

impl Session for ClaudeSession {
    fn turn<'a>(&'a mut self, input: String) -> BoxFuture<'a, Turn> {
        Box::pin(self.run_turn(input))
    }

    fn end(self: Box<Self>, cut: bool) -> BoxFuture<'static, AgentResult> {
        Box::pin(self.close(cut))
    }
}
