//! `gc` stops instances a program left running (it ended without stopping them), by their
//! record: a `run.json` naming a process of the recorded program on the instance's data folder.
//! A process that is not that program on that folder is left alone.
use arena_part::proto::{GcParams, IdParams};
use arena_part::Part;
use arena_sculptured::Sculptured;
use serde_json::json;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The test's folder, removed on drop (with any process the test started).
struct Tmp(PathBuf, Vec<Child>);

impl Drop for Tmp {
    fn drop(&mut self) {
        for c in &mut self.1 {
            // each `sh` leads its group: kill its `sleep` too
            let group = format!("-{}", c.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
            let _ = c.wait();
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A `sh` in a group of its own whose `$0` is `arg`, as serve names its data folder.
fn spawn(arg: &Path) -> Child {
    Command::new("/bin/sh")
        .args(["-c", "sleep 300; :"])
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap()
}

/// Records instance `id` under `saves` as running `/bin/sh` with serve's pid `pid`.
fn record(saves: &Path, id: &str, pid: u32, tmp: &Path) {
    let folder = saves.join(".live").join(id);
    fs::create_dir_all(folder.join("data")).unwrap();
    fs::create_dir_all(tmp).unwrap();
    let run = json!({"config": {"binary": "/bin/sh"}, "binary": "/bin/sh", "build": "",
                     "port": null, "tmp": tmp, "pids": {"serve": pid, "pgid": pid, "hostd": null}});
    fs::write(folder.join("run.json"), run.to_string()).unwrap();
}

fn exited(c: &mut Child) -> bool {
    std::thread::sleep(Duration::from_millis(100));
    c.try_wait().unwrap().is_some()
}

#[tokio::test]
async fn gc_stops_a_leaked_instance_and_nothing_else() {
    let root = std::env::temp_dir().join(format!("arena_sc_gc_{}", std::process::id()));
    let mut t = Tmp(root.clone(), vec![]);
    let saves = root.join("saves/sculptured");
    let live = saves.join(".live");
    t.1.push(spawn(&live.join("leak/data")));
    // its folder's name starts like `leak_x`'s, whose record names it
    t.1.push(spawn(&live.join("leak_x2/data")));
    let (leak, scratch) = (t.1[0].id(), root.join("scratch"));
    record(&saves, "leak", leak, &scratch);
    let impostor = t.1[1].id();
    record(&saves, "leak_x", impostor, &root.join("scratch_x"));
    let part = Sculptured::new(saves.clone());
    part.gc(GcParams { keep: vec![] }).await.unwrap();
    assert!(exited(&mut t.1[0]), "the leaked server is stopped");
    assert!(
        !exited(&mut t.1[1]),
        "a process not on the instance's folder lives on"
    );
    assert!(!live.join("leak").exists() && !scratch.exists());
    assert!(
        !live.join("leak_x").exists(),
        "its record is gone, its process untouched"
    );
    part.stop(IdParams {
        instance: "leak".into(),
    })
    .await
    .unwrap(); // not running: a no-op
}
