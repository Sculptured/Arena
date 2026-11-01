//! The `/proc` side of [`super`].
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{names_folder, Identity};

/// The program `pid` runs, without the " (deleted)" Linux adds once the file is replaced.
fn exe(pid: u32) -> Option<PathBuf> {
    let link = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let text = link.to_string_lossy();
    Some(PathBuf::from(
        text.strip_suffix(" (deleted)").unwrap_or(&text),
    ))
}

fn args(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args = raw.split(|b| *b == 0).filter(|a| !a.is_empty());
    Some(
        args.map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

/// `path` with its links resolved, when it exists.
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `pid` runs `id.exe` with an argument naming `id.folder` (false when it is gone or a
/// zombie, which has no command line).
pub fn is(pid: u32, id: &Identity) -> io::Result<bool> {
    let ours = exe(pid).is_some_and(|e| real(&e) == real(id.exe))
        && args(pid).is_some_and(|a| names_folder(&a, id.folder));
    Ok(ours)
}

/// Kills `pid` when it is `id`: its whole group when it leads group `pgid` (not this program's),
/// else it alone; then waits up to `wait` until it is gone. False when it was not `id` (or gone).
pub fn kill(pid: u32, pgid: Option<u32>, id: &Identity, wait: Duration) -> io::Result<bool> {
    if !is(pid, id)? {
        return Ok(false);
    }
    let p = pid as libc::pid_t;
    // SAFETY: getpgid, killpg and kill only read their integer arguments.
    unsafe {
        let group = libc::getpgid(p);
        let leads = pgid.is_some_and(|g| g as libc::pid_t == group) && group == p;
        if leads && group != libc::getpgid(0) {
            libc::killpg(group, libc::SIGKILL);
        } else {
            libc::kill(p, libc::SIGKILL);
        }
    }
    let deadline = Instant::now() + wait;
    while is(pid, id)? {
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!("process {pid} is still running")));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(true)
}
