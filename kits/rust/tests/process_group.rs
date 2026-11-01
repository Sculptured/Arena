//! The process-group helper: finding programs by their command line and killing their groups.
use arena_part::helpers::process_group::{kill_matching, matching};
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn kill_matching_kills_the_whole_group_and_waits() {
    let tag = format!("arena_kit_pg_{}", std::process::id());
    // a `sh` with the tag on its command line, in a group of its own, and its `sleep` child
    let mut sh = Command::new("sh")
        .args(["-c", &format!("sleep 300 & echo $!; wait # {tag}")])
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut line = String::new();
    let out = sh.stdout.take().unwrap();
    BufReader::new(out).read_line(&mut line).unwrap();
    let sleep = format!("/proc/{}", line.trim());
    let reaper = std::thread::spawn(move || sh.wait());
    assert_eq!(matching(&tag).len(), 1, "the sh carries the tag");
    kill_matching(&tag, Duration::from_secs(5)).unwrap();
    assert!(matching(&tag).is_empty());
    assert!(!reaper.join().unwrap().unwrap().success());
    std::thread::sleep(Duration::from_millis(200)); // init reaps the orphaned sleep
    assert!(!Path::new(&sleep).exists(), "the sleep died with its group");
}

#[test]
fn matching_never_lists_this_process() {
    let me = std::process::id();
    assert!(!matching("process_group").contains(&me));
    kill_matching("arena_kit_pg_nothing_has_this", Duration::from_secs(1)).unwrap();
}
