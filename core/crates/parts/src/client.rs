//! A running part program and its JSON-RPC calls.
use crate::manifest::{Manifest, Run};
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

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
    stdin: AsyncMutex<ChildStdin>,
    timeout: Duration,
    _child: Child,
}

fn substitute(arg: &str, bin_dir: &Path) -> String {
    arg.replace("${ARENA_BIN_DIR}", &bin_dir.to_string_lossy())
}

fn command_for(m: &Manifest, bin_dir: &Path, saves_root: &Path) -> Result<Command> {
    match &m.run {
        Run::Command(argv) => {
            let mut args: Vec<String> = argv.iter().map(|a| substitute(a, bin_dir)).collect();
            if args[0].contains('/') && Path::new(&args[0]).is_relative() {
                args[0] = m.dir.join(&args[0]).to_string_lossy().into_owned();
            }
            let mut c = Command::new(&args[0]);
            c.args(&args[1..]).current_dir(&m.dir).env_clear();
            for k in [
                "PATH",
                "HOME",
                "ARENA_PG_URL",
                "UV_CACHE_DIR",
                "XDG_CACHE_HOME",
                "RUST_LOG",
            ] {
                if let Some(v) = std::env::var_os(k) {
                    c.env(k, v);
                }
            }
            c.env("ARENA_SAVES", saves_root);
            Ok(c)
        }
        Run::Image(image) => {
            let mine = saves_root.join(&m.name);
            std::fs::create_dir_all(&mine)?;
            let mut c = Command::new("docker");
            c.args(["run", "-i", "--rm", "--network"]);
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
            Ok(c)
        }
    }
}

impl Client {
    /// Starts the program. Command mode runs in the manifest's folder with a cleared environment.
    pub async fn spawn(m: &Manifest, bin_dir: &Path, saves_root: &Path) -> Result<Client> {
        let mut cmd = command_for(m, bin_dir, saves_root)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", m.name))?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let state = Arc::new(Mutex::new(State::default()));

        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let tail2 = tail.clone();
        let err_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut t = tail2.lock().unwrap();
                if t.len() == STDERR_LINES {
                    t.pop_front();
                }
                t.push_back(l);
            }
        });
        let state2 = state.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&l) else {
                    continue;
                };
                let Some(id) = v.get("id").and_then(Value::as_u64) else {
                    continue;
                };
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
                if let Some(tx) = state2.lock().unwrap().pending.remove(&id) {
                    let _ = tx.send(reply);
                }
            }
            let _ = err_task.await; // stderr closes with the program
            let msg = tail
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let mut st = state2.lock().unwrap();
            st.stopped = Some(msg.clone());
            for (_, tx) in st.pending.drain() {
                let _ = tx.send(Err(CallError::Stopped(msg.clone())));
            }
        });
        Ok(Client {
            pid,
            next_id: Mutex::new(0),
            state,
            stdin: AsyncMutex::new(stdin),
            timeout: DEFAULT_TIMEOUT,
            _child: child,
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

    /// One request; concurrent calls are matched to replies by id.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, CallError> {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut st = self.state.lock().unwrap();
            if let Some(tail) = &st.stopped {
                return Err(CallError::Stopped(tail.clone()));
            }
            st.pending.insert(id, tx);
        }
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        {
            // A failed write means the program is gone: the reader task then answers with the stderr tail.
            let mut w = self.stdin.lock().await;
            if w.write_all(line.as_bytes()).await.is_ok() {
                let _ = w.flush().await;
            }
        }
        let reply = match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(CallError::Stopped(String::new())),
            Err(_) => {
                self.state.lock().unwrap().pending.remove(&id);
                Err(CallError::Timeout)
            }
        };
        let v = reply?;
        serde_json::from_value(v).map_err(|e| CallError::Part {
            code: -32603,
            message: format!("bad result: {e}"),
        })
    }
}
