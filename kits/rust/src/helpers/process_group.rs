//! Killing programs a part started that live on in process groups of their own (servers that
//! detach their helpers, for one), by the identity the part recorded: a process is killed only
//! when it still runs the recorded program and one of its arguments names a folder the part owns.
//! Linux only (it reads `/proc`); elsewhere every call is an "unsupported" error.
use std::path::Path;
#[cfg(not(target_os = "linux"))]
use std::{io, time::Duration};

/// Who a process must be to be killed: the program it runs and a folder of the part's that one
/// of its arguments is, or is under.
#[derive(Debug, Clone, Copy)]
pub struct Identity<'a> {
    pub exe: &'a Path,
    pub folder: &'a Path,
}

/// True when one whole argument is `folder` or a path under it (`/tmp/x_12` is not under
/// `/tmp/x_1`).
pub fn names_folder<S: AsRef<str>>(args: &[S], folder: &Path) -> bool {
    args.iter()
        .any(|a| Path::new(a.as_ref()).starts_with(folder))
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{is, kill};

/// Whether `pid` runs `id.exe` with an argument naming `id.folder` (false when it is gone).
#[cfg(not(target_os = "linux"))]
pub fn is(_pid: u32, _id: &Identity) -> io::Result<bool> {
    Err(unsupported())
}

/// Kills `pid` when it is `id`: its whole group when it leads group `pgid` (not this program's),
/// else it alone; then waits up to `wait` until it is gone. False when it was not `id` (or gone).
#[cfg(not(target_os = "linux"))]
pub fn kill(_pid: u32, _pgid: Option<u32>, _id: &Identity, _wait: Duration) -> io::Result<bool> {
    Err(unsupported())
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "killing recorded processes is only supported on Linux",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_arguments_only() {
        let folder = Path::new("/tmp/x_12");
        assert!(names_folder(&["serve", "/tmp/x_12"], folder));
        assert!(names_folder(
            &["--data-dir", "/tmp/x_12/data/host-local"],
            folder
        ));
        assert!(!names_folder(&["/tmp/x_123/data"], folder));
        assert!(!names_folder(&["/tmp/x_1"], folder));
        assert!(!names_folder(&["--data-dir=/tmp/x_12/data"], folder));
        assert!(!names_folder(&["sh -c /tmp/x_12"], folder));
        assert!(!names_folder::<&str>(&[], folder));
    }
}
