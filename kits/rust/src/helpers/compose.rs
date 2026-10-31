//! Docker Compose projects as instances: run them, and move their volumes to and from folders.
//!
//! Volumes are copied as tar streams through a throwaway helper container, so files on the host
//! belong to the current user and need no root access to Docker's own folders.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{bail, Context, Result};

/// The image whose `tar` moves volume contents (any image with `tar` will do).
pub const DEFAULT_HELPER: &str = "redis:6.2-alpine";

fn name_ok(s: &str, extra: &[u8]) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || extra.contains(&b)
        })
}

/// A volume name from a compose file: letters, digits, `_` and `-`.
pub fn valid_volume(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Debug, Clone)]
pub struct Project {
    name: String,
    compose_file: PathBuf,
    env: BTreeMap<String, String>,
    helper: String,
}

/// Runs `cmd`, failing with its stderr when it exits unsuccessfully.
fn run(mut cmd: Command, what: &str) -> Result<()> {
    let out = cmd
        .output()
        .with_context(|| format!("{what}: cannot run"))?;
    check(&out, what)
}

fn check(out: &Output, what: &str) -> Result<()> {
    if out.status.success() {
        return Ok(());
    }
    bail!(
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    )
}

/// Runs `from` with its output fed to `to`'s input; both must succeed.
fn pipe(mut from: Command, mut to: Command, what: &str) -> Result<()> {
    let mut producer = from
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{what}: cannot run"))?;
    let stream = producer.stdout.take().expect("producer stdout is piped");
    let consumed = to.stdin(Stdio::from(stream)).stdout(Stdio::null()).output();
    let produced = producer.wait_with_output()?;
    check(&produced, what)?;
    check(
        &consumed.with_context(|| format!("{what}: cannot run"))?,
        what,
    )
}

impl Project {
    /// The project `name` (lowercase letters, digits and `_`) of `compose_file`, run with `env`
    /// added to the environment. `COMPOSE_PROJECT_NAME` is never passed on.
    pub fn new(name: &str, compose_file: &Path, env: &BTreeMap<String, String>) -> Result<Project> {
        if !name_ok(name, b"") {
            bail!("{name:?} is not a project name");
        }
        Ok(Project {
            name: name.into(),
            compose_file: compose_file.into(),
            env: env.clone(),
            helper: DEFAULT_HELPER.into(),
        })
    }

    /// Moves volumes through `image` instead of the default helper.
    pub fn with_helper(mut self, image: &str) -> Project {
        self.helper = image.into();
        self
    }

    fn compose(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("docker");
        cmd.args(["compose", "-p", &self.name, "-f"])
            .arg(&self.compose_file)
            .args(args)
            .env_remove("COMPOSE_PROJECT_NAME")
            .envs(&self.env);
        cmd
    }

    /// Builds and starts the project.
    pub fn up(&self) -> Result<()> {
        run(self.compose(&["up", "-d", "--wait"]), "docker compose up")
    }

    /// Creates the project's volumes and containers without starting them.
    pub fn create(&self) -> Result<()> {
        run(self.compose(&["up", "--no-start"]), "docker compose create")
    }

    pub fn stop(&self) -> Result<()> {
        run(self.compose(&["stop"]), "docker compose stop")
    }

    pub fn start(&self) -> Result<()> {
        run(self.compose(&["start"]), "docker compose start")
    }

    /// Removes the project's containers, networks and volumes.
    pub fn down_with_volumes(&self) -> Result<()> {
        run(self.compose(&["down", "-v"]), "docker compose down")
    }

    /// Docker's name for the project's volume `volume`.
    fn volume_name(&self, volume: &str) -> Result<String> {
        if !valid_volume(volume) {
            bail!("{volume:?} is not a volume name");
        }
        Ok(format!("{}_{volume}", self.name))
    }

    fn helper_tar(&self, volume: &str, flags: &[&str], interactive: bool) -> Result<Command> {
        let mount = format!("{}:/v", self.volume_name(volume)?);
        let mut cmd = Command::new("docker");
        cmd.args(["run", "--rm", "--pull", "never", "--network", "none"]);
        if interactive {
            cmd.arg("-i");
        }
        cmd.args(["--entrypoint", "tar", "-v", &mount, &self.helper]);
        cmd.args(["-C", "/v"]).args(flags);
        Ok(cmd)
    }

    /// Copies the contents of the volume into the existing folder `dest`, as the current user.
    pub fn export_volume(&self, volume: &str, dest: &Path) -> Result<()> {
        let from = self.helper_tar(volume, &["-cf", "-", "."], false)?;
        let mut to = Command::new("tar");
        to.arg("-C").arg(dest).args(["-xf", "-"]);
        pipe(from, to, "export volume")
    }

    /// Fills the volume with the contents of the folder `src`.
    pub fn import_volume(&self, volume: &str, src: &Path) -> Result<()> {
        let mut from = Command::new("tar");
        from.arg("-C").arg(src).args(["-cf", "-", "."]);
        let to = self.helper_tar(volume, &["-xof", "-"], true)?;
        pipe(from, to, "import volume")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(valid_volume("data") && valid_volume("pg-data_1"));
        assert!(!valid_volume("../x") && !valid_volume("a b") && !valid_volume(""));
        let p = |n| Project::new(n, Path::new("c.yml"), &BTreeMap::new());
        assert!(p("arena_t1").is_ok());
        assert!(p("Arena").is_err() && p("a;b").is_err() && p("").is_err());
    }
}
