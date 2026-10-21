//! The third-party daemon steps of a swap: `makoctl reload`, the fcitx5
//! classic UI reload over D-Bus, restarts of the OSD and night-light
//! units, and the scootnotify look-scoped reload (design section 4.7,
//! tickets `look-leaves-daemons` and `notify-look-reload`).
//!
//! Every step here is best-effort and reported: a missing binary, a daemon
//! that is not running, or a failed signal is a note, never a refusal.
//! The fragments these signals activate live in the rendered look (`mako`,
//! `notify.toml`, `wob.ini`, `fcitx5/theme.conf`, `nightlight`); the Nix
//! side points each app at the state link, so by the time these run the
//! link already names the new files.
//!
//! A private look state (a headless or `--private-look` session) runs none
//! of them: unlike the compositor and bar reloads, which address the
//! session's own sockets, these signals always reach the ambient session,
//! so running them from a test would move the user's real desktop.
//!
//! The scootnotify reload is the control-socket equivalent of
//! `scootnotify msg reload --look` (protocol 2, scoped to the look layer,
//! keeping cards, history and DND): one JSON line each way over the
//! daemon's socket, with the scope receipt checked like the bar's. A
//! daemon that is not running is a note, never a refusal; a refusal or a
//! stall is a loud note, never a hang (the client's 30 s bound). When the
//! mako leaf already reloaded, the daemon is mako, so no scootnotify
//! reload is attempted at all (the leaf already handles mako).
//!
//! The commands, verified against the pinned sources:
//!
//! - `makoctl reload`, guarded so it can never start mako: makoctl
//!   calls `Reload` on `org.freedesktop.Notifications` (mako 1.11.0
//!   `makoctl.c`), and mako ships D-Bus activation for exactly that
//!   name (`fr.emersion.mako.service`: `Name=org.freedesktop.Notifications`),
//!   so a bare reload with mako down would launch it. The swap instead
//!   calls the same `Reload` method through
//!   `busctl --user --auto-start=no` (same bus, same method makoctl
//!   would call), falling back to `makoctl reload` only where no busctl
//!   exists to probe with.
//! - `busctl --user --auto-start=no call org.fcitx.Fcitx5 /controller
//!   org.fcitx.Fcitx.Controller1 ReloadAddonConfig s classicui`: fcitx5
//!   5.1.21 re-reads the classic UI theme (`dbusmodule.cpp`: the service
//!   is `org.fcitx.Fcitx5`, the object `/controller`). `--auto-start=no`
//!   matters: busctl auto-starts the service by default and fcitx5 ships
//!   a D-Bus activation file, so a swap without it would launch fcitx5
//!   the user never started. `fcitx5-remote -r` does not restyle it.
//! - `systemctl --user try-restart scoot-osd` / `scoot-nightlight`:
//!   `try-restart` restarts a running unit and no-ops when it is not
//!   running (a swap never starts what the user never started).

use std::path::Path;
use std::time::Duration;

use super::toolkit::find_on_path;

/// The OSD unit (wob): the name the Nix module gives it, restarted on
/// swap so the new `wob.ini` loads. wob reads its config once, at start.
pub const UNIT_OSD: &str = "scoot-osd";
/// The night-light unit: restarted on swap so the wrapper reads the new
/// temperature at start.
pub const UNIT_NIGHTLIGHT: &str = "scoot-nightlight";

/// What the leaf steps did: who reloaded, and the notes for the reply.
#[derive(Debug, Default)]
pub struct LeafReport {
    pub reloaded: Vec<String>,
    pub notes: Vec<String>,
}

/// How long a scootnotify reload waits on a daemon that accepted the
/// connection but does not answer, before the swap stops waiting with a
/// note rather than hanging with the lock held. The daemon's own client
/// uses the same 30 s bound.
pub(crate) const NOTIFY_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest scootnotify reply read. Replies are one line (`ok` with its
/// scope receipt, or `error`); this only bounds a daemon gone wrong.
const NOTIFY_MAX_REPLY: u64 = 64 * 1024;

/// The control protocol the reload speaks: a daemon speaking anything else
/// refuses with a mismatch error, which the leaf reports loudly rather
/// than misreading.
const NOTIFY_PROTOCOL: u32 = 2;

/// The leaf backends, so tests script the daemons.
pub trait Leaves {
    /// `makoctl reload`.
    fn mako_reload(&self) -> Result<(), String>;
    /// The scootnotify look-scoped reload (`msg reload --look` over its
    /// control socket). A missing daemon is an `Err` like any other leaf
    /// failure (a note, never a refusal); `apply_leaves` only calls it
    /// when the mako leaf did not already reload.
    fn notify_reload(&self) -> Result<(), String>;
    /// The fcitx5 `ReloadAddonConfig "classicui"` D-Bus call.
    fn fcitx5_reload(&self) -> Result<(), String>;
    /// `systemctl --user try-restart <unit>`.
    fn unit_try_restart(&self, unit: &str) -> Result<(), String>;
    /// `SIGUSR1` to the user's own `hx` processes: Helix 25.07.1
    /// re-reads its config and theme from disk on it (verified against
    /// its 25.07.1 source and live). Returns how many were signaled.
    fn helix_signal(&self) -> Result<usize, String>;
    /// `SIGUSR2` to the user's own `btop` processes: btop 1.4.7
    /// re-reads its config and theme on it (verified against its 1.4.7
    /// source and live). Returns how many were signaled.
    fn btop_signal(&self) -> Result<usize, String>;
}

/// The real backends: `makoctl`, `busctl` and `systemctl` on `PATH`.
pub struct CliLeaves {
    makoctl: Option<std::path::PathBuf>,
    busctl: Option<std::path::PathBuf>,
    systemctl: Option<std::path::PathBuf>,
    ps: Option<std::path::PathBuf>,
}

impl CliLeaves {
    pub fn new() -> Self {
        Self {
            makoctl: find_on_path("makoctl"),
            busctl: find_on_path("busctl"),
            systemctl: find_on_path("systemctl"),
            ps: find_on_path("ps"),
        }
    }
}

impl Default for CliLeaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Leaves for CliLeaves {
    fn mako_reload(&self) -> Result<(), String> {
        // Never D-Bus-activate mako: `makoctl reload` addresses
        // `org.freedesktop.Notifications`, for which mako ships an
        // activation file, so a bare call with mako down would launch a
        // daemon the user never started. `busctl --auto-start=no` calls
        // the same `Reload` method makoctl would (`fr.emersion.Mako` on
        // `/fr/emersion/Mako`, mako 1.11.0 `makoctl.c`/`dbus/mako.c`)
        // without activating anything; only where no busctl exists at
        // all does `makoctl reload` remain the fallback.
        if let Some(busctl) = self.busctl.as_ref() {
            let out = super::apply::run_with_timeout(
                busctl,
                &[
                    "--user",
                    "--auto-start=no",
                    "call",
                    "org.freedesktop.Notifications",
                    "/fr/emersion/Mako",
                    "fr.emersion.Mako",
                    "Reload",
                ],
            )?;
            if out.status.success() {
                return Ok(());
            }
            return Err(mako_bus_error(&out));
        }
        let program = self
            .makoctl
            .as_ref()
            .ok_or_else(|| "no `makoctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["reload"])?;
        if out.status.success() {
            Ok(())
        } else {
            Err("`makoctl reload` failed (mako not running?)".to_owned())
        }
    }

    fn fcitx5_reload(&self) -> Result<(), String> {
        let program = self
            .busctl
            .as_ref()
            .ok_or_else(|| "no `busctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(
            program,
            &[
                "--user",
                "--auto-start=no",
                "call",
                "org.fcitx.Fcitx5",
                "/controller",
                "org.fcitx.Fcitx.Controller1",
                "ReloadAddonConfig",
                "s",
                "classicui",
            ],
        )?;
        if out.status.success() {
            Ok(())
        } else {
            Err("fcitx5 `ReloadAddonConfig` failed (fcitx5 not running?)".to_owned())
        }
    }

    fn notify_reload(&self) -> Result<(), String> {
        let socket = notify_socket_path(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        )?;
        notify_look_reload(&socket)
    }

    fn unit_try_restart(&self, unit: &str) -> Result<(), String> {
        let program = self
            .systemctl
            .as_ref()
            .ok_or_else(|| "no `systemctl` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["--user", "try-restart", unit])?;
        if out.status.success() {
            Ok(())
        } else {
            // A unit that was never installed (or no systemd to own it) is
            // skipped, not reported: an absent daemon is not an error.
            // Anything else (a real restart failure) stays a note.
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.lines().next().unwrap_or("").trim();
            let detail = if detail.chars().count() > 160 {
                format!("{}…", detail.chars().take(160).collect::<String>())
            } else {
                detail.to_owned()
            };
            if is_unit_absent(&format!(
                "{detail} {}",
                String::from_utf8_lossy(&out.stdout)
            )) {
                return Err(UNIT_ABSENT.to_owned());
            }
            if detail.is_empty() {
                Err(format!("`systemctl --user try-restart {unit}` failed"))
            } else {
                Err(format!(
                    "`systemctl --user try-restart {unit}` failed: {detail}"
                ))
            }
        }
    }

    fn helix_signal(&self) -> Result<usize, String> {
        self.signal_own("hx", rustix::process::Signal::USR1)
    }

    fn btop_signal(&self) -> Result<usize, String> {
        self.signal_own("btop", rustix::process::Signal::USR2)
    }
}

impl CliLeaves {
    /// Sends `signal` to every process named exactly `name` owned by this
    /// user, returning how many were signaled. The process list comes
    /// from `ps -e -o pid= -o uid= -o comm=` (POSIX spellings, on Linux
    /// and macOS alike): no `pkill` dialect, no shell, and the uid
    /// filter is this code's, not the matcher's default, so another
    /// user's same-named process is never touched even as root. The
    /// name compares against the command's basename, unwrapping a
    /// Nix-style `.<name>-wrapped` launcher (see [`leaf_comm`]):
    /// exactly, so `hx-something` never matches `hx`. A pid
    /// that exits between the listing and the kill is skipped silently
    /// (`ESRCH`); any other kill failure is reported, never a refusal
    /// (every leaf step is best-effort). Our own pid is never signaled.
    fn signal_own(&self, name: &str, signal: rustix::process::Signal) -> Result<usize, String> {
        let program = self
            .ps
            .as_ref()
            .ok_or_else(|| "no `ps` on PATH".to_owned())?;
        let table = ps_table(program)?;
        let uid = rustix::process::getuid().as_raw();
        let own = std::process::id();
        let mut signaled = 0;
        let mut failures = Vec::new();
        for line in table.lines() {
            let Some((pid, owner, comm)) = parse_ps_line(line) else {
                continue;
            };
            if !leaf_comm(name, &comm) || owner != uid || pid == own {
                continue;
            }
            let Some(target) = rustix::process::Pid::from_raw(pid as i32) else {
                continue;
            };
            match rustix::process::kill_process(target, signal) {
                // Delivered. (A pid that exits between the listing and
                // the kill reads as `ESRCH` below: it existed when
                // listed, so the attempt counts all the same.)
                Ok(()) => signaled += 1,
                Err(rustix::io::Errno::SRCH) => signaled += 1,
                Err(error) => failures.push(format!("pid {pid}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(signaled)
        } else {
            Err(failures.join("; "))
        }
    }
}

/// The largest `ps` table read: process tables are kilobytes (tens on a
/// busy box); anything past this is a runaway, killed, never buffered.
const PS_MAX_BYTES: usize = 16 << 20;

/// Runs `ps` for the process table, draining its stdout as it goes. The
/// table exceeds the pipe buffer on a busy box (72 KiB measured), so
/// collecting after exit — the way
/// [`run_with_timeout`](super::apply::run_with_timeout) does — would
/// deadlock the child on a full pipe instead. Nonblocking reads pump up
/// to the cap under the same helper deadline; a stale `ps` is killed
/// and reported, never waited on forever.
fn ps_table(ps: &Path) -> Result<String, String> {
    use std::io::Read as _;
    use std::os::unix::io::AsFd as _;
    let mut child = std::process::Command::new(ps)
        .args(["-e", "-o", "pid=", "-o", "uid=", "-o", "comm="])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot run `{}`: {error}", ps.display()))?;
    let mut stdout = child.stdout.take().expect("ps stdout is piped");
    rustix::fs::fcntl_setfl(
        stdout.as_fd(),
        rustix::fs::fcntl_getfl(stdout.as_fd())
            .map_err(|error| format!("cannot poll `{}`: {error}", ps.display()))?
            | rustix::fs::OFlags::NONBLOCK,
    )
    .map_err(|error| format!("cannot poll `{}`: {error}", ps.display()))?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 65536];
    let deadline = std::time::Instant::now() + super::apply::HELPER_TIMEOUT;
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > PS_MAX_BYTES {
                    child.kill().ok();
                    child.wait().ok();
                    return Err(format!(
                        "`{}` wrote past {PS_MAX_BYTES} bytes: killed",
                        ps.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                match child.try_wait() {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if std::time::Instant::now() >= deadline {
                            child.kill().ok();
                            child.wait().ok();
                            return Err(format!(
                                "`{}` timed out after {}s",
                                ps.display(),
                                super::apply::HELPER_TIMEOUT.as_secs()
                            ));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(error) => {
                        return Err(format!("cannot run `{}`: {error}", ps.display()));
                    }
                }
            }
            Err(error) => {
                return Err(format!("cannot run `{}`: {error}", ps.display()));
            }
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("cannot run `{}`: {error}", ps.display()))?;
    if !status.success() {
        return Err(format!("`{}` failed", ps.display()));
    }
    String::from_utf8(out).map_err(|error| format!("`{}` wrote non-UTF-8: {error}", ps.display()))
}

/// Whether `comm` names the leaf's process: the command's basename,
/// exactly (`comm` carries the full path on macOS, the bare name on
/// Linux), unwrapping a Nix-style `.<name>-wrapped` launcher. nixpkgs
/// wraps binaries with a launcher that execs `.<name>-wrapped` beside
/// it, and the kernel reports the inner name: Helix on NixOS runs as
/// `.hx-wrapped` (verified live), btop as `btop`. A longer name that
/// merely starts the same (`hx-something`) never matches.
fn leaf_comm(name: &str, comm: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or_default();
    if base == name {
        return true;
    }
    base.strip_prefix('.')
        .and_then(|inner| inner.strip_suffix("-wrapped"))
        == Some(name)
}

/// One `ps -e -o pid= -o uid= -o comm=` line: the pid, the owner's uid,
/// and the command (a bare name on Linux, a full path on macOS — the
/// caller matches on its basename — possibly with spaces, joined
/// defensively). Anything else (a header, a truncated line) is skipped.
fn parse_ps_line(line: &str) -> Option<(u32, u32, String)> {
    let mut words = line.split_whitespace();
    let pid: u32 = words.next()?.parse().ok()?;
    let uid: u32 = words.next()?.parse().ok()?;
    let comm: Vec<&str> = words.collect();
    if comm.is_empty() || pid == 0 {
        return None;
    }
    Some((pid, uid, comm.join(" ")))
}

/// A failed Reload over the bus: mako not running (never activated, so
/// this is just a note), or the bus itself unreachable. The first stderr
/// line carries the cause; over-long output is cut.
fn mako_bus_error(out: &std::process::Output) -> String {
    let detail = String::from_utf8_lossy(&out.stderr)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    let detail = if detail.chars().count() > 160 {
        format!("{}…", detail.chars().take(160).collect::<String>())
    } else {
        detail
    };
    if detail.is_empty() {
        "`busctl` Reload call failed (mako not running?)".to_owned()
    } else {
        format!("`busctl` Reload call failed (mako not running?): {detail}")
    }
}

/// Marker for a unit that does not exist (or no systemd owns units):
/// [`apply_leaves`] swallows it silently, an absent daemon skipped.
pub(crate) const UNIT_ABSENT: &str = "unit absent: skipped";

/// Whether a failed `try-restart` means the unit is simply absent (never
/// installed, or systemd not running): skipped silently, never a note.
fn is_unit_absent(output: &str) -> bool {
    let lower = output.to_lowercase();
    [
        "not found",
        "no such",
        "unknown unit",
        "failed to get unit",
        "not loaded",
        "system has not been booted",
        "not been booted",
        "unit not found",
        "could not find",
        "does not exist",
        "not exist",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// The scootnotify control socket for this session: `$XDG_RUNTIME_DIR/
/// `scootnotify-<NAME>.sock` with `NAME` from `$WAYLAND_DISPLAY` (its
/// final component, sanitized the way the daemon names it; `wayland-0`
/// when unset or empty). Pure over its inputs, so tests pin the paths
/// without touching the process environment. Refuses (as a not-running
/// note, never a hang) when no runtime dir resolves, no display name
/// remains, or the path would not fit a Unix socket.
pub(crate) fn notify_socket_path(
    wayland_display: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> Result<std::path::PathBuf, String> {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = runtime_dir
        .filter(|d| !d.is_empty())
        .map(Path::new)
        .ok_or_else(|| "no XDG_RUNTIME_DIR: no scootnotify socket".to_owned())?;
    if !dir.is_absolute() {
        return Err(format!(
            "XDG_RUNTIME_DIR is not absolute: {}",
            dir.display()
        ));
    }
    // The daemon's own display-name rule (see `scootui::control::paths`):
    // the final `/`-component, any byte outside `[A-Za-z0-9._-]` becomes
    // `_`, so a display can neither escape the runtime dir nor miss the
    // daemon.
    let name = match wayland_display.filter(|v| !v.is_empty()) {
        None => "wayland-0".to_owned(),
        Some(display) => {
            let bytes = display.as_bytes();
            let last = match bytes.iter().rposition(|&b| b == b'/') {
                Some(slash) => &bytes[slash + 1..],
                None => bytes,
            };
            if last.is_empty() {
                return Err("WAYLAND_DISPLAY has no name to derive a socket from".to_owned());
            }
            last.iter()
                .map(|&b| {
                    if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
                        char::from(b)
                    } else {
                        '_'
                    }
                })
                .collect()
        }
    };
    let socket = dir.join(format!("scootnotify-{name}.sock"));
    if socket.as_os_str().len() > 107 {
        return Err(format!(
            "the scootnotify socket path is longer than a Unix socket allows (107 bytes): {}",
            socket.display()
        ));
    }
    Ok(socket)
}

/// One look-scoped reload against the scootnotify daemon at `socket`:
/// `Ok(())` reloaded, `Err` says why (a missing daemon, a refusal with
/// the daemon's own message, a stall). Never hangs past [`NOTIFY_TIMEOUT`],
/// never panics on a malformed reply.
pub(crate) fn notify_look_reload(socket: &Path) -> Result<(), String> {
    notify_look_reload_with_timeout(socket, NOTIFY_TIMEOUT)
}

/// [`notify_look_reload`] with an explicit wait bound (the tests' short
/// timeout fake: a peer that accepts and never answers must read as a
/// stall, never wedge the swap past the bound).
pub(crate) fn notify_look_reload_with_timeout(
    socket: &Path,
    timeout: Duration,
) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(format!("nothing listens on {}: skipped", socket.display()));
        }
        Err(error) => {
            return Err(format!(
                "cannot reach scootnotify on {}: {error}",
                socket.display()
            ));
        }
    };
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|error| format!("cannot set a timeout on {}: {error}", socket.display()))?;
    // The scoped reload, protocol 2: without the scope the daemon would
    // re-read the user file too, applying half-edited keys the scope
    // exists to skip. An old daemon answers the version it speaks with a
    // mismatch error, which the reply arm reports loudly.
    let request =
        format!("{{\"protocol\":{NOTIFY_PROTOCOL},\"type\":\"reload\",\"scope\":\"look\"}}\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("cannot ask scootnotify on {}: {error}", socket.display()))?;
    let mut reader = BufReader::new((&stream).take(NOTIFY_MAX_REPLY));
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|error| {
        format!(
            "scootnotify on {} did not answer: {error}",
            socket.display()
        )
    })?;
    if line.is_empty() {
        return Err(format!(
            "scootnotify on {} closed the connection without replying: it may hold the new look",
            socket.display()
        ));
    }
    let reply: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
        format!(
            "unreadable reply from scootnotify on {}: {error}",
            socket.display()
        )
    })?;
    match reply.get("type").and_then(|kind| kind.as_str()) {
        Some("ok") => {
            // The scoped reload must carry its scope receipt: a plain
            // `ok` answers a different request. Refuse it loudly, never
            // retry as a plain reload (which would apply pending edits).
            match reply.get("scope").and_then(|scope| scope.as_str()) {
                Some("look") => Ok(()),
                _ => Err(
                    "scootnotify answered the look reload without its scope receipt: refusing it"
                        .to_owned(),
                ),
            }
        }
        Some("error") => {
            let message = reply
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("(no message)");
            Err(format!("scootnotify refused the look reload: {message}"))
        }
        other => Err(format!(
            "scootnotify answered the look reload with an unexpected reply: {}",
            other.unwrap_or("(no type)")
        )),
    }
}

/// Runs the leaf steps for a swap over the rendered `merged` look.
/// Returns who reloaded and the notes. Never fails the swap.
pub fn apply_leaves(
    state_dir: &Path,
    merged: &scoot_look::ResolvedLook,
    leaves: &dyn Leaves,
) -> LeafReport {
    let mut report = LeafReport::default();
    if super::toolkit::is_private_state(state_dir) {
        report.notes.push(
            "private look state (headless or --private-look): third-party daemons left alone"
                .to_owned(),
        );
        return report;
    }
    let mako_active = match leaves.mako_reload() {
        Ok(()) => {
            report.reloaded.push("mako".to_owned());
            true
        }
        Err(error) => {
            report.notes.push(format!("mako not reloaded: {error}"));
            false
        }
    };
    // The scootnotify daemon owns the same bus name mako would: when the
    // mako leaf already reloaded, the daemon is mako, so no scootnotify
    // reload is attempted. Otherwise the running scootnotify (if any)
    // re-reads its look layer here, keeping cards, history and DND.
    if !mako_active {
        match leaves.notify_reload() {
            Ok(()) => report.reloaded.push("scootnotify".to_owned()),
            Err(error) => report
                .notes
                .push(format!("scootnotify not reloaded: {error}")),
        }
    }
    match leaves.fcitx5_reload() {
        Ok(()) => report.reloaded.push("fcitx5".to_owned()),
        Err(error) => report.notes.push(format!("fcitx5 not reloaded: {error}")),
    }
    match leaves.unit_try_restart(UNIT_OSD) {
        Ok(()) => report.reloaded.push(format!("{UNIT_OSD} unit")),
        Err(error) if error == UNIT_ABSENT => {}
        Err(error) => report.notes.push(format!("wob not restarted: {error}")),
    }
    match merged.night_temperature {
        Some(temperature) => match leaves.unit_try_restart(UNIT_NIGHTLIGHT) {
            Ok(()) => report
                .reloaded
                .push(format!("{UNIT_NIGHTLIGHT} unit ({temperature}K)")),
            Err(error) if error == UNIT_ABSENT => {}
            Err(error) => report.notes.push(format!(
                "nightlight not restarted (look wants {temperature}K): {error}"
            )),
        },
        None => report
            .notes
            .push("the look names no night temperature: nightlight left alone".to_owned()),
    }
    report
}

#[cfg(test)]
mod tests;

/// A scripted [`Leaves`] for tests: each backend pops its next scripted
/// result (unscripted calls succeed), recording every unit restart and
/// every editor signal. Shared by the leaf tests and the
/// apply-transaction tests.
#[cfg(test)]
pub(crate) struct ScriptedLeaves {
    mako: std::cell::RefCell<Vec<Result<(), String>>>,
    notify: std::cell::RefCell<Vec<Result<(), String>>>,
    fcitx5: std::cell::RefCell<Vec<Result<(), String>>>,
    units: std::cell::RefCell<Vec<Result<(), String>>>,
    restarted: std::cell::RefCell<Vec<String>>,
    helix: std::cell::RefCell<Vec<Result<usize, String>>>,
    btop: std::cell::RefCell<Vec<Result<usize, String>>>,
    signaled: std::cell::RefCell<Vec<String>>,
}

#[cfg(test)]
impl ScriptedLeaves {
    pub(crate) fn ok() -> Self {
        Self {
            mako: std::cell::RefCell::new(vec![Ok(())]),
            // A mako success means the daemon is mako, so `apply_leaves`
            // never calls the notify backend: script it failing so a stray
            // call is loud. Tests for the notify path script mako failing.
            notify: std::cell::RefCell::new(vec![Err(
                "notify backend called while mako is active".to_owned(),
            )]),
            fcitx5: std::cell::RefCell::new(vec![Ok(())]),
            units: std::cell::RefCell::new(vec![Ok(()), Ok(())]),
            restarted: std::cell::RefCell::new(Vec::new()),
            helix: std::cell::RefCell::new(vec![Ok(1)]),
            btop: std::cell::RefCell::new(vec![Ok(1)]),
            signaled: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            mako: std::cell::RefCell::new(vec![Err("no `makoctl` on PATH".to_owned())]),
            notify: std::cell::RefCell::new(vec![Err("no daemon listens: skipped".to_owned())]),
            fcitx5: std::cell::RefCell::new(vec![Err("no `busctl` on PATH".to_owned())]),
            units: std::cell::RefCell::new(vec![
                Err("no `systemctl` on PATH".to_owned()),
                Err("no `systemctl` on PATH".to_owned()),
            ]),
            restarted: std::cell::RefCell::new(Vec::new()),
            helix: std::cell::RefCell::new(vec![Err("no `ps` on PATH".to_owned())]),
            btop: std::cell::RefCell::new(vec![Err("no `ps` on PATH".to_owned())]),
            signaled: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn restarted(&self) -> Vec<String> {
        self.restarted.borrow().clone()
    }

    pub(crate) fn signaled(&self) -> Vec<String> {
        self.signaled.borrow().clone()
    }

    /// Scripts the next editor signals (popped per call, like every
    /// other backend): `Ok(n)` signals `n` processes, `Err` fails the
    /// step into a note.
    pub(crate) fn script_signals(
        &self,
        helix: Vec<Result<usize, String>>,
        btop: Vec<Result<usize, String>>,
    ) {
        self.helix.replace(helix);
        self.btop.replace(btop);
    }

    /// Scripts the next scootnotify reloads (popped per call): `Ok(())`
    /// reloaded, `Err` fails the step into a note.
    pub(crate) fn script_notify(&self, notify: Vec<Result<(), String>>) {
        self.notify.replace(notify);
    }
}

#[cfg(test)]
impl Leaves for ScriptedLeaves {
    fn mako_reload(&self) -> Result<(), String> {
        self.mako.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn notify_reload(&self) -> Result<(), String> {
        self.notify.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn fcitx5_reload(&self) -> Result<(), String> {
        self.fcitx5.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn unit_try_restart(&self, unit: &str) -> Result<(), String> {
        self.restarted.borrow_mut().push(unit.to_owned());
        self.units.borrow_mut().pop().unwrap_or(Ok(()))
    }

    fn helix_signal(&self) -> Result<usize, String> {
        self.signaled.borrow_mut().push("helix".to_owned());
        self.helix.borrow_mut().pop().unwrap_or(Ok(1))
    }

    fn btop_signal(&self) -> Result<usize, String> {
        self.signaled.borrow_mut().push("btop".to_owned());
        self.btop.borrow_mut().pop().unwrap_or(Ok(1))
    }
}
