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

fn sway_bin() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("SCOOTBG_TEST_SWAY").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("sway"))
        .find(|candidate| candidate.is_file())
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
    /// For a sway session: `swaymsg`'s expected path and sway's IPC socket.
    sway_ipc: Option<(PathBuf, PathBuf)>,
}

impl Session {
    /// Starts `scoot --headless --outputs 2`, or returns `None` (after
    /// saying why) when there is no `scoot` binary and it is not required.
    pub fn start(tag: &str) -> Option<Self> {
        Self::start_with(tag, 2, "")
    }

    /// Starts `scoot --headless --outputs N` with `config` as its config
    /// file; `None` as for [`Session::start`].
    pub fn start_with(tag: &str, outputs: u32, config: &str) -> Option<Self> {
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
        let config_path = scratch.0.join("config.toml");
        fs::write(&config_path, config).unwrap();
        let config = config_path;
        let log = fs::File::create(scratch.0.join("compositor.log")).unwrap();
        let compositor = Command::new(&scoot)
            .args(["--headless", "--outputs", &outputs.to_string(), "--socket"])
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
            sway_ipc: None,
        };
        session.wayland_display = session.wait_for_wayland(|dir| {
            let ipc = dir.join("scoot.sock");
            is_socket(&ipc).then_some(ipc)
        });
        Some(session)
    }

    /// Starts a headless sway (pixman, no GPU, no input devices, one
    /// output), or returns `None` (after saying why) when there is no sway
    /// and it is not required.
    ///
    /// The binary is `$SCOOTBG_TEST_SWAY` if set, else `sway` on `PATH`; a
    /// headless instance in a scratch `XDG_RUNTIME_DIR` never touches a
    /// running session. `SCOOTBG_REQUIRE_SWAY` turns the skip into a
    /// failure, as `SCOOTBG_REQUIRE_SCOOT` does for scoot.
    pub fn sway(tag: &str) -> Option<Self> {
        let Some(sway) = sway_bin() else {
            if std::env::var_os("SCOOTBG_REQUIRE_SWAY").is_some() {
                panic!(
                    "SCOOTBG_REQUIRE_SWAY is set but there is no sway: put it on PATH or \
                     set SCOOTBG_TEST_SWAY"
                );
            }
            eprintln!("skipped -- no sway on PATH (or set SCOOTBG_TEST_SWAY)");
            return None;
        };
        let scratch = Scratch::new(tag);
        let config = scratch.0.join("sway.conf");
        // No swaybg (it would be a second background client), no Xwayland.
        fs::write(&config, "swaybg_command -\nxwayland disable\n").unwrap();
        let log = fs::File::create(scratch.0.join("compositor.log")).unwrap();
        let mut command = Command::new(&sway);
        command
            .arg("-c")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &scratch.0)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("SWAYSOCK")
            .env_remove("DISPLAY")
            // A bus address, even one nothing listens on: nixpkgs' wrapper
            // otherwise starts sway under `dbus-run-session`, which needs a
            // system dbus config. A headless test sway needs no bus.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", scratch.0.join("no-bus").display()),
            )
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log);
        // Its own process group: a packaged sway may run under a wrapper
        // (nixpkgs' starts it through `dbus-run-session`), and killing the
        // group reaches sway itself.
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let compositor = command
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start {}: {e}", sway.display()));
        let mut session = Self {
            scratch,
            compositor: Some(compositor),
            wayland_display: String::new(),
            sway_ipc: None,
        };
        let mut ipc = None;
        session.wayland_display = session.wait_for_wayland(|dir| {
            let found = fs::read_dir(dir)
                .ok()?
                .filter_map(Result::ok)
                .find_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    (name.starts_with("sway-ipc.") && is_socket(&entry.path()))
                        .then(|| entry.path())
                });
            ipc.clone_from(&found);
            found
        });
        session.sway_ipc = ipc.map(|socket| (sway.with_file_name("swaymsg"), socket));
        Some(session)
    }

    /// `swaymsg ARGS` against this session's sway; panics unless it
    /// succeeds.
    pub fn swaymsg(&self, args: &[&str]) -> String {
        let (swaymsg, socket) = self.sway_ipc.as_ref().expect("not a sway session");
        // Beside sway when packaged together, else on PATH.
        let program = if swaymsg.is_file() {
            swaymsg.clone()
        } else {
            PathBuf::from("swaymsg")
        };
        let output = Command::new(program)
            .arg("-s")
            .arg(socket)
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "swaymsg {args:?}: {stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }

    /// Waits until the compositor's Wayland socket and its IPC socket
    /// (found by `ipc`) are both up.
    fn wait_for_wayland(&mut self, mut ipc: impl FnMut(&Path) -> Option<PathBuf>) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            // Both sockets: the Wayland one comes first and the IPC socket
            // once the compositor is fully up.
            let ipc_up = ipc(&self.scratch.0).is_some();
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
                panic!(
                    "the compositor exited during start-up ({status}):\n{}",
                    self.log()
                );
            }
            assert!(
                Instant::now() < deadline,
                "the compositor did not come up within {PATIENCE:?}:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.scratch.0.join("compositor.log")).unwrap_or_default()
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
        let mut command = self.scootbg();
        command.arg("daemon");
        self.start_daemon(command)
    }

    /// Starts `scootbg daemon` with `env` added and its stderr written to
    /// [`Session::daemon_log`] rather than a pipe, which a protocol trace
    /// (`WAYLAND_DEBUG`) could fill, and waits until it answers.
    pub fn daemon_logged(&self, env: &[(&str, &str)]) -> Child {
        let log = fs::File::create(self.daemon_log()).unwrap();
        let mut command = self.scootbg();
        command.arg("daemon").envs(env.iter().copied());
        let mut child = command.stdout(Stdio::null()).stderr(log).spawn().unwrap();
        let deadline = Instant::now() + PATIENCE;
        while !answers(&self.socket()) {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "scootbg daemon exited during start-up ({status}): {}",
                    fs::read_to_string(self.daemon_log()).unwrap_or_default()
                );
            }
            assert!(Instant::now() < deadline, "the daemon never answered");
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    }

    /// Where [`Session::daemon_logged`] writes the daemon's stderr.
    pub fn daemon_log(&self) -> PathBuf {
        self.scratch.0.join("scootbg.log")
    }

    /// `scootbg query`'s reply, parsed.
    pub fn query(&self) -> serde_json::Value {
        let output = self.run(&["query"]);
        assert!(
            output.status.success(),
            "query: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Queries until `done` holds for the output list, failing after
    /// [`PATIENCE`] with the last reply. Returns the list.
    pub fn query_until(
        &self,
        what: &str,
        mut done: impl FnMut(&[serde_json::Value]) -> bool,
    ) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let reply = self.query();
            let outputs = reply["outputs"].as_array().cloned().unwrap_or_default();
            if done(&outputs) {
                return outputs;
            }
            assert!(
                Instant::now() < deadline,
                "never {what}; last reply: {reply}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// One request to scoot's own control socket (a scoot session only),
    /// its reply parsed.
    pub fn scoot_ipc(&self, request: &str) -> serde_json::Value {
        let stream = UnixStream::connect(self.scoot_socket()).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        (&stream).write_all(request.as_bytes()).unwrap();
        (&stream).write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("scoot replied {line:?}: {e}"))
    }

    fn start_daemon(&self, mut command: Command) -> Child {
        let mut child = command
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

    /// Kills the compositor (SIGKILL: it gets no chance to say goodbye),
    /// and for sway its whole process group (see [`Session::sway`]).
    pub fn kill_compositor(&mut self) {
        if let Some(mut compositor) = self.compositor.take() {
            if self.sway_ipc.is_some() {
                let group = rustix::process::Pid::from_child(&compositor);
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            }
            let _ = compositor.kill();
            let _ = compositor.wait();
        }
    }

    /// The `scoot` control socket (a scoot session only).
    pub fn scoot_socket(&self) -> PathBuf {
        self.scratch.0.join("scoot.sock")
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
