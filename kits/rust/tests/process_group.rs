//! The process-group helper: a recorded process is killed, group and all, only when it is still
//! the recorded program with an argument naming the part's folder.
use arena_part::helpers::process_group::{is, kill, Identity};
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// A `sh` naming `folder` (its `$0`), in a group of its own, and the pid of its `sleep` child.
fn spawn_in(folder: &Path) -> (Child, u32) {
    let mut sh = Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 300 & echo $!; wait")
        .arg(folder.join("data"))
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(sh.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    (sh, line.trim().parse().unwrap())
}

/// `pid` (a `sh` on `folder`) is not taken for another program or for a folder whose name
/// only starts or ends alike, and is not killed as one.
fn refused_for_others(pid: u32, folder: &Path) {
    let other_exe = Identity {
        exe: Path::new("/bin/sleep"),
        folder,
    };
    assert!(!is(pid, &other_exe).unwrap(), "another program");
    assert!(!kill(pid, Some(pid), &other_exe, Duration::from_secs(1)).unwrap());
    let longer = PathBuf::from(format!("{}3", folder.display()));
    let shorter = PathBuf::from(format!("/tmp/arena_kit_pg_{}", std::process::id() / 10));
    for f in [&longer, &shorter] {
        let wrong = Identity {
            exe: Path::new("/bin/sh"),
            folder: f,
        };
        assert!(!is(pid, &wrong).unwrap(), "{f:?} is not the folder");
        assert!(!kill(pid, Some(pid), &wrong, Duration::from_secs(1)).unwrap());
    }
}

#[test]
fn only_the_recorded_program_on_the_folder_is_killed() {
    let folder = PathBuf::from(format!("/tmp/arena_kit_pg_{}", std::process::id()));
    let (mut sh, sleep) = spawn_in(&folder);
    let pid = sh.id();
    let reaper = std::thread::spawn(move || sh.wait());
    let ours = Identity {
        exe: Path::new("/bin/sh"),
        folder: &folder,
    };
    assert!(is(pid, &ours).unwrap());
    refused_for_others(pid, &folder);
    assert!(is(pid, &ours).unwrap(), "still running");
    assert!(kill(pid, Some(pid), &ours, Duration::from_secs(5)).unwrap());
    assert!(!reaper.join().unwrap().unwrap().success());
    std::thread::sleep(Duration::from_millis(200)); // init reaps the orphaned sleep
    let sleep_gone = !Path::new(&format!("/proc/{sleep}")).exists();
    assert!(sleep_gone, "the sleep died with its group");
    let again = kill(pid, Some(pid), &ours, Duration::from_secs(1)).unwrap();
    assert!(!again, "already gone");
}
