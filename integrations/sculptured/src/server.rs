//! One `sculptured serve` per instance: started in a process group of its own on a loopback port
//! the system picks (`--port 0`, so never 4410), with the instance's data folder, home and
//! scratch folder, and nothing else from this program's environment. serve starts its runner
//! (`sculptured hostd`) in a session of its own, so stopping kills every process whose command
//! line names the data folder: serve, then its runner.
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arena_part::helpers::process_group::kill_matching;

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

/// Starts `binary serve` on `f`'s data folder and waits until its runner is connected; returns
/// its port. On failure the server is killed and the error quotes its log.
pub async fn launch(binary: &Path, f: &Folders) -> Result<u16> {
    let exited = spawn(binary, f)?;
    let up = async {
        let port = poll::until("the server listening", || port_of(f, &exited)).await?;
        let api = Api::new(port)?;
        poll::until("its runner connected", || runner_connected(&api)).await?;
        anyhow::Ok(port)
    };
    match up.await {
        Ok(port) => Ok(port),
        Err(e) => {
            let _ = kill(f.data()).await;
            Err(e.context(format!("sculptured serve: {}", log_tail(&f.log()))))
        }
    }
}

/// Spawns serve; the returned flag is set once it exits (a thread reaps it).
fn spawn(binary: &Path, f: &Folders) -> Result<Arc<AtomicBool>> {
    let home = f.root.join("home");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&f.tmp)?;
    let _ = std::fs::remove_file(url_file(&f.data())); // a stale one would name an old port
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
    let mut child = cmd.spawn().with_context(|| format!("running {binary:?}"))?;
    let exited = Arc::new(AtomicBool::new(false));
    let flag = exited.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::SeqCst);
    });
    Ok(exited)
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

/// Kills every process whose command line names the data folder `data` (serve and its runner)
/// and waits until they are gone.
pub async fn kill(data: PathBuf) -> Result<()> {
    let needle = data.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || kill_matching(&needle, KILL_WAIT))
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
