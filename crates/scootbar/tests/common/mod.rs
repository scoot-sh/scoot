//! A headless compositor for scootbar to run against, one per test:
//! scootbg's harness (`crates/scootbg/tests/common`), trimmed to what a bar
//! with no control socket needs. What the bar did is read from the
//! compositor: scoot's `outputs` (the usable area its zone leaves), sway's
//! workspaces, and screenshots of both.
//!
//! Each [`Session`] gets its own scratch directory as `XDG_RUNTIME_DIR`, so
//! no two tests' sockets meet (nextest runs each test in its own process;
//! `cargo test` runs them side by side).
//!
//! The compositor binary is `$SCOOTBAR_TEST_SCOOT` if set, else the `scoot`
//! beside this test's `scootbar` in the same target directory (a workspace
//! build puts it there); sway is `$SCOOTBAR_TEST_SWAY`, else `sway` on
//! `PATH`. Without one the test prints why and passes as skipped, unless
//! `SCOOTBAR_REQUIRE_SCOOT` (or `SCOOTBAR_REQUIRE_SWAY`) is set, which turns
//! the skip into a failure so CI cannot go green by skipping.

#![allow(dead_code)] // Each test binary uses a different subset.

mod shots;

/// The seven-segment test font (`src/testfont.rs`), built in code: the
/// bar draws with it here, and the tests read the time back off
/// screenshots.
#[rustfmt::skip]
#[path = "../../src/testfont.rs"]
pub mod testfont;

#[allow(unused_imports)] // Only the binaries that check pixels use it.
pub use shots::{Shot, rgb};

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, so this only bounds a wedge.
pub const PATIENCE: Duration = Duration::from_secs(20);

pub fn scootbar_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootbar"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn scoot_bin() -> Option<PathBuf> {
    env_path("SCOOTBAR_TEST_SCOOT").or_else(|| {
        let sibling = scootbar_bin().with_file_name("scoot");
        sibling.is_file().then_some(sibling)
    })
}

fn sway_bin() -> Option<PathBuf> {
    env_path("SCOOTBAR_TEST_SWAY").or_else(|| on_path("sway"))
}

/// Says why a test is skipped, or panics when `require` is set.
fn skip(require: &str, why: &str) {
    if std::env::var_os(require).is_some() {
        panic!("{require} is set but {why}");
    }
    eprintln!("skipped -- {why}");
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
            std::env::temp_dir().join(format!("sbart-{}-{nanos:08x}-{tag}", std::process::id()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        Self(dir)
    }

    /// The test font, written into this directory on first use: what any
    /// `scootbar daemon` a test starts is pointed at with `--font`, so no
    /// test depends on the fonts the machine has installed (the daemon
    /// loads its font before it connects to anything, and refuses to start
    /// without one).
    pub fn font(&self) -> PathBuf {
        let path = self.0.join("seven.ttf");
        if !path.is_file() {
            fs::write(&path, testfont::build()).unwrap();
        }
        path
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
    /// Starts `scoot --headless --outputs N` with `config` as its config
    /// file, or returns `None` (after saying why) when there is no `scoot`
    /// binary and it is not required.
    pub fn scoot(tag: &str, outputs: u32, config: &str) -> Option<Self> {
        let Some(scoot) = scoot_bin() else {
            skip(
                "SCOOTBAR_REQUIRE_SCOOT",
                &format!(
                    "there is no scoot binary beside {} (build `-p scoot`, or set \
                     SCOOTBAR_TEST_SCOOT)",
                    scootbar_bin().display()
                ),
            );
            return None;
        };
        let scratch = Scratch::new(tag);
        let config_path = scratch.0.join("config.toml");
        fs::write(&config_path, config).unwrap();
        let log = fs::File::create(scratch.0.join("compositor.log")).unwrap();
        let compositor = Command::new(&scoot)
            .args(["--headless", "--outputs", &outputs.to_string()])
            .args(["--width", "1600", "--height", "1000"])
            .arg("--socket")
            .arg(scratch.0.join("scoot.sock"))
            .arg("--config")
            .arg(&config_path)
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

    /// Starts a headless sway (pixman, no GPU, no input devices) with
    /// `outputs` outputs (0 is allowed: outputs come later), or `None` as
    /// for [`Session::scoot`].
    pub fn sway(tag: &str, outputs: u32) -> Option<Self> {
        let Some(sway) = sway_bin() else {
            skip(
                "SCOOTBAR_REQUIRE_SWAY",
                "there is no sway on PATH (or set SCOOTBAR_TEST_SWAY)",
            );
            return None;
        };
        let scratch = Scratch::new(tag);
        let config = scratch.0.join("sway.conf");
        // No swaybg, no Xwayland, no gaps or borders to reason about.
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
            .env("WLR_HEADLESS_OUTPUTS", outputs.to_string())
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("SWAYSOCK")
            .env_remove("DISPLAY")
            // A bus address, even one nothing listens on: nixpkgs' wrapper
            // otherwise starts sway under `dbus-run-session`.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", scratch.0.join("no-bus").display()),
            )
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log);
        // Its own process group, so killing the group reaches sway under a
        // wrapper too.
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

    /// Sway's outputs: `(name, usable rect)` of each, the rect being its
    /// active workspace's, which sway arranges windows within (the output
    /// less every exclusive zone on it).
    pub fn sway_usable(&self) -> Vec<(String, Value)> {
        let workspaces: Value =
            serde_json::from_str(&self.swaymsg(&["-t", "get_workspaces", "-r"])).unwrap();
        let outputs: Value =
            serde_json::from_str(&self.swaymsg(&["-t", "get_outputs", "-r"])).unwrap();
        let mut list = Vec::new();
        for output in outputs.as_array().unwrap() {
            let name = output["name"].as_str().unwrap().to_owned();
            let current = output["current_workspace"].as_str();
            let rect = workspaces
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["name"].as_str() == current)
                .map(|w| w["rect"].clone())
                .unwrap_or(Value::Null);
            list.push((name, rect));
        }
        list
    }

    /// Waits until the compositor's Wayland socket and its IPC socket
    /// (found by `ipc`) are both up.
    fn wait_for_wayland(&mut self, mut ipc: impl FnMut(&Path) -> Option<PathBuf>) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
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

    /// `scootbar` with this session's environment, and none of the
    /// Wayland variables of the one the tests run in.
    pub fn scootbar(&self) -> Command {
        let mut command = Command::new(scootbar_bin());
        command
            .env("XDG_RUNTIME_DIR", &self.scratch.0)
            .env("WAYLAND_DISPLAY", &self.wayland_display)
            .env_remove("WAYLAND_SOCKET")
            .env_remove("WAYLAND_DEBUG")
            .stdin(Stdio::null());
        command
    }

    /// Starts `scootbar daemon ARGS`, its stderr in [`Session::bar_log`].
    /// It has no socket to answer on: tests wait for what it did to show
    /// in the compositor.
    ///
    /// Always with `--font` the test font (so no test depends on the
    /// machine's fonts), and, unless `args` places modules itself, with
    /// none (`--center=`): a solid bar, whose geometry the tests check
    /// pixel by pixel. The clock's tests place it.
    pub fn bar(&self, args: &[&str]) -> Child {
        self.bar_with_env(args, &[])
    }

    /// [`Session::bar`] with `env` added (`WAYLAND_DEBUG=1` for a protocol
    /// trace in [`Session::bar_log`]).
    pub fn bar_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Child {
        let log = fs::File::create(self.bar_log()).unwrap();
        let placed = args.iter().any(|arg| {
            ["--left", "--center", "--right"]
                .iter()
                .any(|flag| arg.starts_with(flag))
        });
        let mut command = self.scootbar();
        command.arg("daemon").arg("--font").arg(self.font());
        if !placed {
            command.arg("--center=");
        }
        command
            .args(args)
            .envs(env.iter().copied())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap()
    }

    /// The test font, written into the scratch directory on first use.
    pub fn font(&self) -> PathBuf {
        self.scratch.font()
    }

    pub fn bar_log(&self) -> PathBuf {
        self.scratch.0.join("scootbar.log")
    }

    pub fn bar_stderr(&self) -> String {
        fs::read_to_string(self.bar_log()).unwrap_or_default()
    }

    /// One request to scoot's own control socket (a scoot session only),
    /// its reply parsed.
    pub fn scoot_ipc(&self, request: &str) -> Value {
        let stream = UnixStream::connect(self.scratch.0.join("scoot.sock")).unwrap();
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        (&stream).write_all(request.as_bytes()).unwrap();
        (&stream).write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("scoot replied {line:?}: {e}"))
    }

    /// scoot's `outputs` list.
    pub fn scoot_outputs(&self) -> Vec<Value> {
        let reply = self.scoot_ipc(r#"{"type":"outputs"}"#);
        reply["outputs"].as_array().cloned().unwrap_or_default()
    }

    /// Polls `check` until it returns `Some`, failing after [`PATIENCE`]
    /// with `what` and the bar's stderr; `bar` is checked for an early exit.
    pub fn wait_for<T>(
        &self,
        bar: &mut Child,
        what: &str,
        mut check: impl FnMut(&Self) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = check(self) {
                return found;
            }
            if let Some(status) = bar.try_wait().unwrap() {
                panic!("{what}: scootbar exited ({status}): {}", self.bar_stderr());
            }
            assert!(
                Instant::now() < deadline,
                "never {what}; scootbar said: {}",
                self.bar_stderr()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Kills the compositor (SIGKILL), and for sway its whole process group.
    /// Its children go first: a client scoot spawned (`foot`, through an IPC
    /// `spawn`) is re-parented to init when scoot dies, and would otherwise
    /// outlive the test.
    pub fn kill_compositor(&mut self) {
        if let Some(mut compositor) = self.compositor.take() {
            kill_children(compositor.id());
            if self.sway_ipc.is_some() {
                let group = rustix::process::Pid::from_child(&compositor);
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            }
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

/// SIGKILLs every process whose parent is `parent`, and waits (a bounded
/// while) until none is left.
pub fn kill_children(parent: u32) {
    let children = || -> Vec<u32> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .filter(|&pid| {
                fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        let after = stat.rsplit_once(')')?.1;
                        after.split_whitespace().nth(1)?.parse::<u32>().ok()
                    })
                    == Some(parent)
            })
            .collect()
    };
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = children();
        if left.is_empty() || Instant::now() >= deadline {
            return;
        }
        for pid in left {
            if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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
            panic!("scootbar did not exit within {PATIENCE:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Kills a bar still running at the end of a test.
pub struct Reaper(pub Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Every `wl_shm_pool.create_buffer` in a `WAYLAND_DEBUG` trace, as
/// `(width, height)`, in order.
pub fn created_buffers(trace: &str) -> Vec<(u32, u32)> {
    trace
        .lines()
        .filter_map(|line| {
            let args = line.split(".create_buffer(").nth(1)?;
            let mut fields = args.split(", ").skip(2);
            let width = fields.next()?.parse().ok()?;
            let height = fields.next()?.parse().ok()?;
            Some((width, height))
        })
        .collect()
}

/// The voluntary context switches of every thread of `pid`: its wakeups.
pub fn wakeups(pid: u32) -> u64 {
    let mut total = 0;
    for task in fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
        let status = fs::read_to_string(task.unwrap().path().join("status")).unwrap();
        total += status
            .lines()
            .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
            .map(|v| v.trim().parse::<u64>().unwrap())
            .unwrap();
    }
    total
}

/// The `wl_shm` buffers `pid` has mapped: `scootbg_mem`'s memfds (named
/// for scootbg, whose crate they come from). Their fds are closed once the
/// pool exists, so a leaked buffer shows here and not in [`open_fds`].
pub fn shm_mappings(pid: u32) -> usize {
    fs::read_to_string(format!("/proc/{pid}/maps"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("/memfd:scootbg-wallpaper"))
        .count()
}

/// The number of fds `pid` holds open.
pub fn open_fds(pid: u32) -> usize {
    fs::read_dir(format!("/proc/{pid}/fd")).unwrap().count()
}
