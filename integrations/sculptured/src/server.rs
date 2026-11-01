//! One `sculptured serve` per instance: started in a process group of its own on a loopback port
//! the system picks (`--port 0`, so never 4410), with the instance's data folder, home and
//! scratch folder, and nothing else from this program's environment. serve starts its runner
//! (`sculptured hostd`) in a session of its own, so both are recorded (`Pids`) and stopping kills
//! each, group and all, only while it is still the program with an argument naming the data
//! folder: serve first (it would start another runner), then the runner.
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arena_part::helpers::process_group::{self, Identity};
use serde::{Deserialize, Serialize};

use crate::api::Api;
use crate::poll;

/// How long a stop waits for the killed processes to go.
const KILL_WAIT: Duration = Duration::from_secs(10);
/// The last bytes of the server's log quoted when it fails to come up.
const LOG_TAIL: usize = 2000;

/// The folders of one instance's server.
pub struct Folders {
    /// The instance folder: `data/`, `home/` and `serve.log` are in it.
    pub root: PathBuf,
    /// The scratch folder (`TMPDIR`), short enough for the runner's socket path.
    pub tmp: PathBuf,
}

impl Folders {
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn log(&self) -> PathBuf {
        self.root.join("serve.log")
    }
}

/// The processes of a server: serve (leading its own group) and its runner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Pids {
    pub serve: Option<u32>,
    pub pgid: Option<u32>,
    pub hostd: Option<u32>,
}

/// A serve just started: its processes, and a flag set once it exits (a thread reaps it).
pub struct Spawned {
    pub pids: Pids,
    exited: Arc<AtomicBool>,
}

/// The `--version` of `binary` (`sculptured 0.1.0`): the build a save records.
pub fn version(binary: &Path) -> Result<String> {
    let out = Command::new(binary).arg("--version").env_clear().output();
    let out = out.with_context(|| format!("running {binary:?} --version"))?;
    if !out.status.success() {
        bail!("{binary:?} --version failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Spawns `binary serve` on `f`'s data folder in a process group of its own.
pub fn spawn(binary: &Path, f: &Folders) -> Result<Spawned> {
    let _ = std::fs::remove_file(url_file(&f.data())); // a stale one would name an old port
    let mut child = command(binary, f)?
        .spawn()
        .with_context(|| format!("running {binary:?}"))?;
    let pid = child.id();
    let exited = Arc::new(AtomicBool::new(false));
    let flag = exited.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::SeqCst);
    });
    let pids = Pids {
        serve: Some(pid),
        pgid: Some(pid),
        hostd: None,
    };
    Ok(Spawned { pids, exited })
}

/// The serve command: its own group, its own folders and log, a cleared environment.
fn command(binary: &Path, f: &Folders) -> Result<Command> {
    let home = f.root.join("home");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&f.tmp)?;
    let log = File::create(f.log())?;
    let mut cmd = Command::new(binary);
    cmd.arg("serve")
        .args(["--port", "0", "--data-dir"])
        .arg(f.data());
    cmd.arg("--web-dir").arg(f.root.join("web")); // none: no web UI
    cmd.env_clear().env("HOME", &home).env("TMPDIR", &f.tmp);
    cmd.env("RUST_LOG", "warn");
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd.current_dir(&f.root).stdin(Stdio::null());
    cmd.stdout(log.try_clone()?).stderr(log).process_group(0);
    Ok(cmd)
}

/// A server ready for requests: its port, and its runner's pid.
pub struct Ready {
    pub port: u16,
    pub hostd: Option<u32>,
}

/// Waits until the server listens and its runner is connected. The error quotes its log.
pub async fn ready(f: &Folders, s: &Spawned) -> Result<Ready> {
    let up = async {
        let port = poll::until("the server listening", || port_of(f, &s.exited)).await?;
        let api = Api::new(port)?;
        poll::until("its runner connected", || runner_connected(&api)).await?;
        anyhow::Ok(Ready {
            port,
            hostd: hostd_pid(&f.data()),
        })
    };
    let log = f.log();
    up.await
        .map_err(|e| e.context(format!("sculptured serve: {}", log_tail(&log))))
}

/// The pid the runner wrote to `host-local/hostd.pid`.
fn hostd_pid(data: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(data.join("host-local/hostd.pid")).ok()?;
    text.trim().parse().ok()
}

/// Where serve writes its runner's address, `ws://127.0.0.1:<port>/host`, once it listens.
fn url_file(data: &Path) -> PathBuf {
    data.join("host-local/controld.url")
}

async fn port_of(f: &Folders, exited: &AtomicBool) -> Result<Option<u16>> {
    if exited.load(Ordering::SeqCst) {
        bail!("it exited");
    }
    let Ok(url) = std::fs::read_to_string(url_file(&f.data())) else {
        return Ok(None);
    };
    let port = url
        .trim()
        .strip_prefix("ws://127.0.0.1:")
        .and_then(|r| r.strip_suffix("/host"))
        .and_then(|p| p.parse().ok());
    match port {
        Some(p) => Ok(Some(p)),
        None => bail!("unexpected runner address {url:?}"),
    }
}

/// Whether the local runner is among the connected hosts.
async fn runner_connected(api: &Api) -> Result<Option<()>> {
    let Ok(reply) = api.get(&["hosts"], &[]).await else {
        return Ok(None); // not answering yet
    };
    let hosts = reply.expect("the host list")?;
    let local = hosts
        .as_array()
        .is_some_and(|a| a.iter().any(|h| h["id"] == "local"));
    Ok(local.then_some(()))
}

/// Kills the recorded processes of the server of `binary` on the data folder `data`, and the
/// runner its `hostd.pid` names now, each only while it is still that program on that folder,
/// and waits until they are gone.
pub async fn kill(binary: PathBuf, data: PathBuf, pids: Pids) -> Result<()> {
    let task = move || {
        let id = Identity {
            exe: &binary,
            folder: &data,
        };
        if let Some(serve) = pids.serve {
            process_group::kill(serve, pids.pgid, &id, KILL_WAIT)?;
        }
        let mut runners = vec![pids.hostd, hostd_pid(&data)];
        runners.dedup();
        for hostd in runners.into_iter().flatten() {
            process_group::kill(hostd, Some(hostd), &id, KILL_WAIT)?;
        }
        std::io::Result::Ok(())
    };
    tokio::task::spawn_blocking(task)
        .await?
        .context("stopping sculptured")
}

fn log_tail(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let start = text.len().saturating_sub(LOG_TAIL);
    let start = (start..text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    text[start..].trim().to_string()
}
