//! The clone's private scratch directory, the watchdog that cleans up
//! after a scoot that dies, the directory size meter and the escaped
//! rendering of git's own words.

use std::fs::File;
use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use super::SCRATCH_MAX_ENTRIES;
use super::url::sanitize;

/// What the watchdog runs: remember the last process-group id scoot sent
/// on stdin, and when stdin closes (scoot returned, or was killed by a
/// signal, Ctrl-C included) kill that group if it is still registered and
/// remove the scratch directory. `$1` is the directory; it is an argument,
/// never spliced into the script.
const WATCHDOG: &str = r#"last=0
while read -r pg; do last=$pg; done
if [ "$last" -gt 0 ] 2>/dev/null; then kill -KILL -- "-$last" 2>/dev/null; fi
rm -rf -- "$1""#;

/// A private scratch directory, removed when dropped, with a watchdog
/// that removes it (and stops a running git) if scoot itself is killed.
pub struct Scratch {
    path: PathBuf,
    watchdog: Option<Watchdog>,
}

struct Watchdog {
    child: Child,
    stdin: Option<ChildStdin>,
}

#[cfg(test)]
thread_local! {
    /// Every group id [`Scratch::register_group`] was given on this thread,
    /// in order, so a test can see the runner's wiring without killing
    /// anything. Per thread: parallel tests never see each other's calls.
    pub static REGISTERED: std::cell::RefCell<Vec<u32>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

impl Scratch {
    /// Creates `scoot-look-clone-<pid>-<nanos>-<n>` under `base` with mode
    /// 0700. `mkdir` fails on anything already there (a planted link
    /// included), and the name is retried, so an existing path is never
    /// reused.
    pub fn new(base: &Path) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt as _;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        for attempt in 0..16u32 {
            let path = base.join(format!(
                "scoot-look-clone-{}-{nanos}-{attempt}",
                std::process::id()
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let watchdog = Watchdog::spawn(&path);
                    return Ok(Self { path, watchdog });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "cannot make a private directory under {}: {error}",
                        base.display()
                    ));
                }
            }
        }
        Err(format!(
            "cannot make a private directory under {}: every name taken",
            base.display()
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Tells the watchdog which process group is running git now (0 for
    /// none). Best effort: without a watchdog this does nothing.
    pub fn register_group(&mut self, group: u32) {
        #[cfg(test)]
        REGISTERED.with(|log| log.borrow_mut().push(group));
        if let Some(stdin) = self.watchdog.as_mut().and_then(|w| w.stdin.as_mut()) {
            let _ = writeln!(stdin, "{group}");
        }
    }

    /// Closes the watchdog's pipe without removing the directory, as the
    /// death of scoot would. Tests only.
    #[cfg(test)]
    pub fn simulate_death(&mut self) -> Option<Child> {
        let mut watchdog = self.watchdog.take()?;
        drop(watchdog.stdin.take());
        Some(watchdog.child)
    }
}

impl Watchdog {
    /// Starts the watchdog in its own process group, so a terminal's
    /// Ctrl-C (which goes to scoot's group only) does not stop it. A box
    /// with no `sh` runs without one: the normal exit paths still clean up.
    fn spawn(dir: &Path) -> Option<Self> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(WATCHDOG)
            .arg("scoot-look-watchdog")
            .arg(dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .ok()?;
        let stdin = child.stdin.take();
        Some(Self { child, stdin })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        if let Some(mut watchdog) = self.watchdog.take() {
            drop(watchdog.stdin.take());
            let _ = watchdog.child.wait();
        }
    }
}

/// Whether the tree under `root` holds more than `cap` bytes (or more than
/// [`SCRATCH_MAX_ENTRIES`] entries). Never follows a link.
pub fn dir_exceeds(root: &Path, cap: u64) -> bool {
    let mut total: u64 = 0;
    let mut entries = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            entries += 1;
            if entries > SCRATCH_MAX_ENTRIES {
                return true;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
                if total > cap {
                    return true;
                }
            }
        }
    }
    false
}

/// Bytes read from each end of a long stderr file.
const STDERR_END_BYTES: u64 = 2048;
/// Non-empty lines kept from the start and from the end.
const KEEP_HEAD: usize = 3;
const KEEP_TAIL: usize = 3;

/// What git said, for an error: the first [`KEEP_HEAD`] and last
/// [`KEEP_TAIL`] non-empty lines (ssh's own diagnostic comes first, git's
/// summary last, and both matter), every character that could drive a
/// terminal replaced by `?`, each line and the whole bounded. Remote
/// servers write some of these lines, so none reaches the terminal raw.
pub fn stderr_summary(path: &Path) -> String {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut raw = Vec::new();
    if len <= 2 * STDERR_END_BYTES {
        if file.read_to_end(&mut raw).is_err() {
            return String::new();
        }
    } else {
        let mut tail = Vec::new();
        let ok = (&mut file)
            .take(STDERR_END_BYTES)
            .read_to_end(&mut raw)
            .is_ok()
            && file.seek(SeekFrom::Start(len - STDERR_END_BYTES)).is_ok()
            && file.take(STDERR_END_BYTES).read_to_end(&mut tail).is_ok();
        if !ok {
            return String::new();
        }
        raw.push(b'\n');
        raw.extend_from_slice(&tail);
    }
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.chars().take(200).map(sanitize).collect::<String>())
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() <= KEEP_HEAD + KEEP_TAIL {
        return lines.join("; ");
    }
    let mut kept: Vec<&str> = lines[..KEEP_HEAD].iter().map(String::as_str).collect();
    kept.push("...");
    kept.extend(lines[lines.len() - KEEP_TAIL..].iter().map(String::as_str));
    kept.join("; ")
}
