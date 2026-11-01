//! Programs a part started that live on in process groups of their own (servers that detach
//! their helpers, for one): found by their command line, killed group and all.
use std::io;
use std::time::{Duration, Instant};

/// The processes (but this one) whose command line holds `needle`. Zombies have none.
pub fn matching(needle: &str) -> Vec<u32> {
    let me = std::process::id();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    rd.filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| pid != me && cmdline(pid).is_some_and(|c| c.contains(needle)))
        .collect()
}

/// The command line of `pid`, its arguments joined by spaces.
fn cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&raw).replace('\0', " "))
}

/// Kills (SIGKILL) the process group of every process whose command line holds `needle`, and
/// waits up to `wait` until none is left. A process in this program's own group is killed alone.
pub fn kill_matching(needle: &str, wait: Duration) -> io::Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        let found = matching(needle);
        if found.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "processes {found:?} ({needle}) are still running"
            )));
        }
        found.into_iter().for_each(kill_group_of);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn kill_group_of(pid: u32) {
    let pid = pid as libc::pid_t;
    // SAFETY: getpgid, killpg and kill only read their integer arguments.
    unsafe {
        let group = libc::getpgid(pid);
        if group > 0 && group != libc::getpgid(0) {
            libc::killpg(group, libc::SIGKILL);
        } else {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}
