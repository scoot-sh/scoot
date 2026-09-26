//! A headless scoot for scootbg to run against, one per test.
//!
//! Each [`Session`] gets its own scratch directory as `XDG_RUNTIME_DIR`, so
//! the compositor's Wayland socket, scoot's control socket and scootbg's
//! socket and lock never meet another test's (nextest runs each test in its
//! own process; `cargo test` runs them side by side).
//!
//! The compositor binary is `$SCOOTBG_TEST_SCOOT` if set, else the `scoot`
//! beside this test's `scootbg` in the same target directory (a workspace
//! build, `cargo nextest run --workspace` included, puts it there). When
//! neither exists the test prints why and passes as skipped, unless
//! `SCOOTBG_REQUIRE_SCOOT` is set, which turns the skip into a failure, so
//! CI's integration job cannot go green by skipping (the same arrangement
//! as `SCOOT_REQUIRE_XWAYLAND`).

#![allow(dead_code)] // Each test binary uses a different subset.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, so this only bounds a wedge.
pub const PATIENCE: Duration = Duration::from_secs(20);

pub fn scootbg_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootbg"))
}

fn scoot_bin() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("SCOOTBG_TEST_SCOOT").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let sibling = scootbg_bin().with_file_name("scoot");
    sibling.is_file().then_some(sibling)
}

/// A scratch directory, removed on drop.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        // Short: the Unix socket paths inside must fit 107 bytes.
        let dir =
            std::env::temp_dir().join(format!("sbgt-{}-{nanos:08x}-{tag}", std::process::id()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Session {
    pub scratch: Scratch,
    compositor: Option<Child>,
    pub wayland_display: String,
}

impl Session {
    /// Starts `scoot --headless`, or returns `None` (after saying why) when
    /// there is no `scoot` binary and it is not required.
    pub fn start(tag: &str) -> Option<Self> {
        let Some(scoot) = scoot_bin() else {
            if std::env::var_os("SCOOTBG_REQUIRE_SCOOT").is_some() {
                panic!(
                    "SCOOTBG_REQUIRE_SCOOT is set but there is no scoot binary: build it \
                     (`cargo build -p scoot`) or set SCOOTBG_TEST_SCOOT"
                );
            }
            eprintln!(
                "skipped -- no scoot binary beside {} (build `-p scoot`, or set \
                 SCOOTBG_TEST_SCOOT)",
                scootbg_bin().display()
            );
            return None;
        };
        let scratch = Scratch::new(tag);
        let config = scratch.0.join("config.toml");
        fs::write(&config, "").unwrap();
        let log = fs::File::create(scratch.0.join("scoot.log")).unwrap();
        let compositor = Command::new(&scoot)
            .args(["--headless", "--outputs", "2", "--socket"])
            .arg(scratch.0.join("scoot.sock"))
            .arg("--config")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &scratch.0)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start {}: {e}", scoot.display()));
        let mut session = Self {
            scratch,
            compositor: Some(compositor),
            wayland_display: String::new(),
        };
        session.wayland_display = session.wait_for_wayland();
        Some(session)
    }

    /// The first `wayland-N` socket scoot binds in the fresh directory.
    fn wait_for_wayland(&mut self) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            // Both sockets: scoot binds the Wayland one first and its IPC
            // socket once it is fully up.
            let ipc_up = is_socket(&self.scratch.0.join("scoot.sock"));
            let wayland = fs::read_dir(&self.scratch.0).ok().and_then(|entries| {
                entries.filter_map(Result::ok).find_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    (name.starts_with("wayland-") && is_socket(&entry.path())).then_some(name)
                })
            });
            if let (true, Some(name)) = (ipc_up, wayland) {
                return name;
            }
            if let Some(status) = self.compositor.as_mut().and_then(|c| c.try_wait().unwrap()) {
                panic!("scoot exited during start-up ({status}):\n{}", self.log());
            }
            assert!(
                Instant::now() < deadline,
                "scoot did not come up within {PATIENCE:?}:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.scratch.0.join("scoot.log")).unwrap_or_default()
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.scratch.0
    }

    pub fn socket(&self) -> PathBuf {
        self.scratch
            .0
            .join(format!("scootbg-{}.sock", self.wayland_display))
    }

    /// `scootbg` with this session's environment.
    pub fn scootbg(&self) -> Command {
        let mut command = Command::new(scootbg_bin());
        command
            .env("XDG_RUNTIME_DIR", &self.scratch.0)
            .env("WAYLAND_DISPLAY", &self.wayland_display)
            .env_remove("WAYLAND_SOCKET")
            .stdin(Stdio::null());
        command
    }

    /// `scootbg ARGS`, run to completion.
    pub fn run(&self, args: &[&str]) -> Output {
        self.scootbg().args(args).output().unwrap()
    }

    /// Starts `scootbg daemon` and waits until it answers a request, which
    /// means its poll loop is running (the socket file alone appears before
    /// the Wayland handshake, and a stale one is there before the daemon).
    pub fn daemon(&self) -> Child {
        let mut child = self
            .scootbg()
            .arg("daemon")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + PATIENCE;
        while !answers(&self.socket()) {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "scootbg daemon exited during start-up ({status}): {}",
                    stderr_of(&mut child)
                );
            }
            assert!(Instant::now() < deadline, "the daemon never answered");
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    }

    /// Kills the compositor (SIGKILL: it gets no chance to say goodbye).
    pub fn kill_compositor(&mut self) {
        if let Some(mut compositor) = self.compositor.take() {
            let _ = compositor.kill();
            let _ = compositor.wait();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill_compositor();
    }
}

/// Whether a daemon answers a `version` request on `socket`.
pub fn answers(socket: &Path) -> bool {
    let Ok(stream) = UnixStream::connect(socket) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let mut line = String::new();
    (&stream)
        .write_all(b"{\"protocol\":1,\"type\":\"version\"}\n")
        .is_ok()
        && BufReader::new(&stream).read_line(&mut line).is_ok()
        && line.contains("\"version\"")
}

pub fn is_socket(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

/// Waits for `child` to exit, failing the test after [`PATIENCE`].
pub fn wait_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the daemon did not exit within {PATIENCE:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whatever the child wrote to stderr (it must have been piped and have
/// exited).
pub fn stderr_of(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut text);
    }
    text
}

/// Sends `signal` to `child`.
pub fn signal(child: &Child, signal: rustix::process::Signal) {
    let pid = rustix::process::Pid::from_child(child);
    rustix::process::kill_process(pid, signal).unwrap();
}
