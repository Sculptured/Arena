//! A running part program and its JSON-RPC calls.
use crate::manifest::{Manifest, Run};
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

const STDERR_LINES: usize = 20;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub enum CallError {
    /// The part answered with a JSON-RPC error.
    Part {
        code: i64,
        message: String,
    },
    /// The program is gone; carries the last 20 stderr lines.
    Stopped(String),
    Timeout,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CallError::Part { code, message } => write!(f, "{message} ({code})"),
            CallError::Stopped(tail) => write!(f, "stopped: {tail}"),
            CallError::Timeout => write!(f, "timed out"),
        }
    }
}

impl std::error::Error for CallError {}

type Reply = Result<Value, CallError>;

#[derive(Default)]
struct State {
    pending: HashMap<u64, oneshot::Sender<Reply>>,
    /// Set once the program's output has closed: the stderr tail.
    stopped: Option<String>,
}

pub struct Client {
    pid: u32,
    next_id: Mutex<u64>,
    state: Arc<Mutex<State>>,
    /// Whole request lines for the single writer task.
    lines: mpsc::UnboundedSender<String>,
    timeout: Duration,
    /// Container name in image mode, removed on drop.
    container: Option<String>,
}

/// Removes a call's pending entry when the call finishes or its future is dropped.
struct PendingGuard<'a> {
    state: &'a Mutex<State>,
    id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.state.lock().unwrap().pending.remove(&self.id);
    }
}

fn kill_group(pid: u32) {
    if pid > 0 {
        // The program leads its own process group, so this takes grandchildren too.
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    }
}

fn substitute(arg: &str, bin_dir: &Path) -> String {
    arg.replace("${ARENA_BIN_DIR}", &bin_dir.to_string_lossy())
}

/// Env vars a command-mode part inherits; everything else is cleared.
const PASSED_ENV: [&str; 6] = [
    "PATH",
    "HOME",
    "ARENA_PG_URL",
    "UV_CACHE_DIR",
    "XDG_CACHE_HOME",
    "RUST_LOG",
];

fn command_for(
    m: &Manifest,
    bin_dir: &Path,
    saves_root: &Path,
    env: &[(String, String)],
) -> Result<(Command, Option<String>)> {
    match &m.run {
        Run::Command(argv) => Ok((program_command(m, argv, bin_dir, saves_root, env), None)),
        Run::Image(image) => {
            let (c, name) = container_command(m, image, saves_root)?;
            Ok((c, Some(name)))
        }
    }
}

/// Command mode: the program runs in the manifest's folder with a cleared environment.
fn program_command(
    m: &Manifest,
    argv: &[String],
    bin_dir: &Path,
    saves_root: &Path,
    env: &[(String, String)],
) -> Command {
    let mut args: Vec<String> = argv.iter().map(|a| substitute(a, bin_dir)).collect();
    if args[0].contains('/') && Path::new(&args[0]).is_relative() {
        args[0] = m.dir.join(&args[0]).to_string_lossy().into_owned();
    }
    let mut c = Command::new(&args[0]);
    c.args(&args[1..]).current_dir(&m.dir).env_clear();
    for k in PASSED_ENV {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.env("ARENA_SAVES", saves_root);
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

/// Image mode: `docker run` with no network (unless the manifest asks for the host's) and only
/// this package's saves mounted. Returns the container's name.
fn container_command(m: &Manifest, image: &str, saves_root: &Path) -> Result<(Command, String)> {
    let mine = saves_root.join(&m.name);
    std::fs::create_dir_all(&mine)?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "arena-part-{}-{}-{}",
        m.name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let mut c = Command::new("docker");
    c.args(["run", "-i", "--rm", "--name", &name, "--network"]);
    if m.needs.network.as_deref() == Some("host") {
        eprintln!("parts: {} runs with host networking", m.name);
        c.arg("host");
    } else {
        c.arg("none");
    }
    c.arg("-v")
        .arg(format!("{}:/saves", mine.display()))
        .arg(image)
        .env_clear();
    if let Some(p) = std::env::var_os("PATH") {
        c.env("PATH", p); // docker itself needs to be found; the container gets nothing
    }
    Ok((c, name))
}

/// One request line to write to the program, at a time, until it is gone.
fn spawn_writer(mut stdin: tokio::process::ChildStdin) -> mpsc::UnboundedSender<String> {
    let (lines, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(l) = rx.recv().await {
            if stdin.write_all(l.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break; // the waiter task reports the exit
            }
        }
    });
    lines
}

type Tail = Arc<Mutex<VecDeque<String>>>;

/// Keeps the last `STDERR_LINES` lines of stderr.
fn spawn_stderr_tail(stderr: tokio::process::ChildStderr) -> (Tail, tokio::task::JoinHandle<()>) {
    let tail = Tail::default();
    let tail2 = tail.clone();
    let task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let mut t = tail2.lock().unwrap();
            if t.len() == STDERR_LINES {
                t.pop_front();
            }
            t.push_back(l);
        }
    });
    (tail, task)
}

/// The reply a response line carries, with the id it answers.
fn parse_reply(line: &str) -> Option<(u64, Reply)> {
    let v = serde_json::from_str::<Value>(line).ok()?;
    let id = v.get("id").and_then(Value::as_u64)?;
    let reply = match v.get("error") {
        Some(e) => Err(CallError::Part {
            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
    };
    Some((id, reply))
}

/// Routes each response line to the call waiting for its id.
fn spawn_reader(stdout: tokio::process::ChildStdout, state: Arc<Mutex<State>>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let Some((id, reply)) = parse_reply(&l) else {
                continue;
            };
            if let Some(tx) = state.lock().unwrap().pending.remove(&id) {
                let _ = tx.send(reply);
            }
        }
    });
}

/// The direct child exiting is what stops the client, even if a grandchild keeps the pipes open.
fn spawn_waiter(
    mut child: tokio::process::Child,
    pid: u32,
    tail: Tail,
    err_task: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<State>>,
) {
    tokio::spawn(async move {
        let _ = child.wait().await;
        kill_group(pid);
        let _ = tokio::time::timeout(Duration::from_secs(2), err_task).await;
        let msg = tail
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let mut st = state.lock().unwrap();
        st.stopped = Some(msg.clone());
        for (_, tx) in st.pending.drain() {
            let _ = tx.send(Err(CallError::Stopped(msg.clone())));
        }
    });
}

impl Client {
    /// Starts the program. Command mode runs in the manifest's folder with a cleared environment.
    pub async fn spawn(m: &Manifest, bin_dir: &Path, saves_root: &Path) -> Result<Client> {
        Client::spawn_with_env(m, bin_dir, saves_root, &[]).await
    }

    /// Like `spawn`, with extra environment variables for command mode (they override the ones
    /// passed through). Image mode ignores them: a container gets no environment from arenad.
    pub async fn spawn_with_env(
        m: &Manifest,
        bin_dir: &Path,
        saves_root: &Path,
        env: &[(String, String)],
    ) -> Result<Client> {
        let (mut cmd, container) = command_for(m, bin_dir, saves_root, env)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", m.name))?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let lines = spawn_writer(stdin);
        let (tail, err_task) = spawn_stderr_tail(stderr);
        spawn_reader(stdout, state.clone());
        spawn_waiter(child, pid, tail, err_task, state.clone());
        Ok(Client {
            pid,
            next_id: Mutex::new(0),
            state,
            lines,
            timeout: DEFAULT_TIMEOUT,
            container,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Client {
        self.timeout = timeout;
        self
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// True once the program has exited.
    pub fn is_stopped(&self) -> bool {
        self.state.lock().unwrap().stopped.is_some()
    }

    /// A fresh call id and the channel its reply arrives on; refuses once the program is gone.
    fn register(&self) -> Result<(u64, oneshot::Receiver<Reply>), CallError> {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = oneshot::channel();
        let mut st = self.state.lock().unwrap();
        if let Some(tail) = &st.stopped {
            return Err(CallError::Stopped(tail.clone()));
        }
        st.pending.insert(id, tx);
        Ok((id, rx))
    }

    /// One request; concurrent calls are matched to replies by id. Gives up (`Timeout`) after the
    /// client's timeout, though the program may still be working on it.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, CallError> {
        self.call_within(method, params, Some(self.timeout)).await
    }

    /// `call` without a deadline: waits until the program answers or exits. For work that must be
    /// over before the caller goes on.
    pub async fn call_until_done<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, CallError> {
        self.call_within(method, params, None).await
    }

    async fn call_within<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        limit: Option<Duration>,
    ) -> Result<T, CallError> {
        let (id, rx) = self.register()?;
        let _guard = PendingGuard {
            state: &self.state,
            id,
        };
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        // A dead program is reported by the waiter task, with the stderr tail.
        let _ = self.lines.send(line);
        let v = wait_for(rx, limit).await?;
        serde_json::from_value(v).map_err(|e| CallError::Part {
            code: -32603,
            message: format!("bad result: {e}"),
        })
    }

    /// Calls waiting for an answer (for tests).
    pub fn pending_len(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }
}

/// The reply on `rx`, waiting at most `limit` (None: until the program answers or exits).
async fn wait_for(rx: oneshot::Receiver<Reply>, limit: Option<Duration>) -> Reply {
    let answered = match limit {
        Some(limit) => match tokio::time::timeout(limit, rx).await {
            Ok(answered) => answered,
            Err(_) => return Err(CallError::Timeout),
        },
        None => rx.await,
    };
    answered.unwrap_or_else(|_| Err(CallError::Stopped(String::new())))
}

impl Drop for Client {
    fn drop(&mut self) {
        kill_group(self.pid);
        if let Some(name) = &self.container {
            // Best effort: a container must not outlive its client.
            if let Ok(mut c) = std::process::Command::new("docker")
                .args(["rm", "-f", name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
            }
        }
    }
}
