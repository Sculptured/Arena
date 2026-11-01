//! The server container, and the MariaDB command line tools run inside it with `docker exec`.
//!
//! Every value goes in as its own argument (no shell); the root password reaches the tools as
//! env `MYSQL_PWD`, passed by name so it never shows in a command line.
use std::io::{self, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;

use anyhow::{bail, Context, Result};

use super::login::Login;
use super::server::{ServerConfig, IMAGE};

/// The label on containers this helper made; any other container is never used.
const LABEL_KEY: &str = "arena.part";
const LABEL_VALUE: &str = "mariadb";
const DATADIR: &str = "/var/lib/mysql";
/// The server's flags: row-based binlog with full row images, kept until purged.
const FLAGS: [&str; 6] = [
    "--log-bin=binlog",
    "--binlog-format=ROW",
    "--binlog-row-image=FULL",
    "--server-id=1",
    "--binlog_expire_logs_seconds=0",
    "--max_binlog_size=64M",
];
/// Applied history and loaded dumps stay out of the binlog: a restored database's own segment
/// starts after them.
const UNLOGGED: &str = "--init-command=SET sql_log_bin=0";

/// What `docker inspect` says about the configured container name.
enum Found {
    Missing,
    Ours { running: bool },
    Foreign,
}

fn docker() -> Command {
    Command::new("docker")
}

/// Runs `cmd` to the end; its stdout, or an error with its stderr.
fn output(mut cmd: Command, what: &str) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("{what}: cannot run docker"))?;
    if !out.status.success() {
        bail!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn inspect(cfg: &ServerConfig) -> Result<Found> {
    let mut cmd = docker();
    let format = format!("{{{{index .Config.Labels \"{LABEL_KEY}\"}}}} {{{{.State.Running}}}}");
    cmd.args(["container", "inspect", "-f", &format, &cfg.container]);
    let out = cmd.output().context("docker inspect: cannot run docker")?;
    if !out.status.success() {
        return Ok(Found::Missing);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(match text.trim().split_once(' ') {
        Some((LABEL_VALUE, running)) => Found::Ours {
            running: running == "true",
        },
        _ => Found::Foreign,
    })
}

/// Starts the server container: creates it (never pulling the image) when it is missing,
/// starts it when it is stopped. Refuses a container of that name that this helper did not make.
pub fn ensure_container(cfg: &ServerConfig) -> Result<()> {
    match inspect(cfg)? {
        Found::Ours { running: true } => Ok(()),
        Found::Ours { running: false } => {
            let mut cmd = docker();
            cmd.args(["start", &cfg.container]);
            output(cmd, "docker start").map(drop)
        }
        Found::Missing => run_container(cfg),
        Found::Foreign => bail!(
            "container {} exists but was not made by arena (no label {LABEL_KEY}={LABEL_VALUE}); \
             configure another container name",
            cfg.container
        ),
    }
}

fn run_container(cfg: &ServerConfig) -> Result<()> {
    let mut cmd = docker();
    cmd.args(["run", "-d", "--pull", "never", "--name", &cfg.container])
        .args(["--label", &format!("{LABEL_KEY}={LABEL_VALUE}")])
        .args(["-v", &format!("{}:{DATADIR}", cfg.volume)])
        .args(["-p", &format!("127.0.0.1:{}:3306", cfg.port)])
        .args(["-e", "MARIADB_ROOT_PASSWORD"])
        .env("MARIADB_ROOT_PASSWORD", &cfg.root_password)
        .arg(IMAGE)
        .args(FLAGS);
    output(cmd, "docker run").map(drop)
}

/// `docker exec -i <container> <tool> -u<user> <args..>` with the password in `MYSQL_PWD`.
fn exec(cfg: &ServerConfig, login: &Login, tool: &str, args: &[&str]) -> Command {
    let mut cmd = docker();
    let user = format!("-u{}", login.user);
    cmd.args(["exec", "-i", "-e", "MYSQL_PWD", &cfg.container, tool, &user])
        .args(args)
        .env("MYSQL_PWD", &login.password)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    cmd
}

/// A started child and a thread collecting its stderr (so a chatty tool never blocks).
struct Running {
    child: Child,
    stderr: JoinHandle<String>,
    what: &'static str,
}

impl Running {
    fn spawn(mut cmd: Command, what: &'static str) -> Result<Running> {
        let mut child = cmd
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("{what}: cannot run docker"))?;
        let mut err = child.stderr.take().expect("stderr was piped");
        let stderr = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        Ok(Running {
            child,
            stderr,
            what,
        })
    }

    fn stdout(&mut self) -> ChildStdout {
        self.child.stdout.take().expect("stdout was piped")
    }

    fn finish(mut self) -> Result<()> {
        let status = self.child.wait()?;
        let stderr = self.stderr.join().unwrap_or_default();
        if !status.success() {
            bail!("{} failed: {}", self.what, stderr.trim());
        }
        Ok(())
    }
}

/// Streams `mariadb-dump` of `db` (as root) into `sink`. The dump notes the binlog position
/// it is consistent with (`--master-data=2`, as a comment).
pub fn dump<T>(
    cfg: &ServerConfig,
    db: &str,
    sink: impl FnOnce(&mut dyn Read) -> io::Result<T>,
) -> Result<T> {
    let args = [
        "--single-transaction",
        "--master-data=2",
        "--skip-dump-date",
        db,
    ];
    let mut cmd = exec(cfg, &Login::root(cfg), "mariadb-dump", &args);
    cmd.stdout(Stdio::piped());
    let mut run = Running::spawn(cmd, "mariadb-dump")?;
    // The pipe closes when `sink` returns, so a failed sink never leaves the dump blocked.
    let got = sink(&mut run.stdout());
    let done = run.finish();
    let got = got.context("storing the dump")?;
    done.map(|()| got)
}

/// Runs the SQL read from `src` in `db` as `login`, unlogged.
pub fn load(cfg: &ServerConfig, login: &Login, db: &str, src: &mut dyn Read) -> Result<()> {
    let mut cmd = exec(cfg, login, "mariadb", &[UNLOGGED, db]);
    cmd.stdin(Stdio::piped());
    let mut run = Running::spawn(cmd, "mariadb")?;
    let mut stdin = run.child.stdin.take().expect("stdin was piped");
    let copied = io::copy(src, &mut stdin);
    drop(stdin);
    run.finish()?;
    copied.map(drop).context("feeding SQL to mariadb")
}

/// What to replay: `db`'s events from `start` in the first file to `stop` in the last, as `into`.
pub struct Replay {
    pub db: String,
    pub into: String,
    pub files: Vec<String>,
    pub start: u64,
    pub stop: u64,
}

/// `mariadb-binlog` over the replay's files, piped into `mariadb` as `login` (unlogged).
pub fn apply_binlog(cfg: &ServerConfig, login: &Login, r: &Replay) -> Result<()> {
    let filter = format!("--database={}", r.db);
    let rewrite = format!("--rewrite-db={}->{}", r.db, r.into);
    let (start, stop) = (
        format!("--start-position={}", r.start),
        format!("--stop-position={}", r.stop),
    );
    let mut args = vec![filter.as_str(), &rewrite, &start, &stop];
    args.extend(r.files.iter().map(String::as_str));
    let mut read = exec(cfg, &Login::root(cfg), "mariadb-binlog", &args);
    read.stdout(Stdio::piped());
    let mut reader = Running::spawn(read, "mariadb-binlog")?;
    let mut apply = exec(cfg, login, "mariadb", &[UNLOGGED, &r.into]);
    apply.stdin(reader.stdout());
    let applier = Running::spawn(apply, "mariadb (applying the binlog)")?;
    let applied = applier.finish();
    reader.finish()?;
    applied
}
