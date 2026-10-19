//! Launching: the activation token's environment, the systemd scope,
//! the double-fork fallback, and `DBusActivatable`.
//!
//! A pick in the panel (or `Ctrl+Enter` on typed text) lands here, after
//! the panel minted an activation token for it
//! (`dev/design/scootlaunch.md`, Launching correctly):
//!
//! 1. **Environment.** From the launcher's own environment: the
//!    launcher's own `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`
//!    are removed (a consumed token must not leak), then the new token
//!    is set as `XDG_ACTIVATION_TOKEN` (and as `DESKTOP_STARTUP_ID`
//!    while `DISPLAY` is set, for X clients). The working directory is
//!    `Path=`, else `$HOME`. stdin is `/dev/null`; stdout and stderr
//!    are inherited; every other launcher fd is close-on-exec (the
//!    Rust `std` default).
//! 2. **`DBusActivatable` first.** When the entry says so, `Activate`
//!    (or `ActivateAction` for a desktop action) goes to
//!    `org.freedesktop.Application` on the bus name from the id, with
//!    `activation-token` in `platform_data`. A refused call falls back
//!    to `Exec`, which the spec still asks entries to carry.
//! 3. **Spawn into a scope.** The child starts, then
//!    `org.freedesktop.systemd1.Manager.StartTransientUnit` moves it
//!    into `app-scootlaunch-<id>-<random>.scope` (in `app.slice`, the
//!    slice systemd's desktop-environment page holds normal
//!    applications in), bounded at 500 ms. The child runs a few
//!    hundred microseconds in the launcher's cgroup before the move.
//! 4. **Fallback.** With no session bus at all, the child starts
//!    detached instead: a double fork with `setsid`, so the app is out
//!    of the launcher's session and process group and is reparented.
//!    It stays in the compositor's cgroup, and the docs say so. A
//!    failed scope move after a direct spawn is not this path: the app
//!    keeps running where it started, with one warning.
//! 5. **Record** frecency, and the panel closes.
//!
//! Nothing here blocks the panel's close past its bounds: the token
//! wait is 200 ms and every bus round trip shares one 500 ms deadline.
//! A token is never written to a log or a `Debug` line (it is a bearer
//! credential for someone else's focus decision); failures name the
//! erring step, never the token.
//!
//! No `unsafe` here. The bus calls use the workspace's hand-rolled
//! client shapes (`scoot_dbus::proto`) over a plain socket with
//! timeouts, so a stopped bus costs the bound, never a hang.

#[cfg(test)]
mod tests;

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{BUS_INTERFACE, BUS_NAME, BUS_PATH, BusAddr, bus_addr};
use scoot_dbus::proto::{Kind, Message, Writer, check_name, frame_at};
use scoot_rt::print::warn;

use crate::index::{Action, Entry};
use crate::rank::frecency::load;

/// How long the panel waits for the token's `done` after `commit`:
/// a local compositor answers in milliseconds, so this only bounds a
/// wedge. Past it the launch goes without a token (a fresh window
/// still focuses on map).
pub const TOKEN_TIMEOUT: Duration = Duration::from_millis(200);
/// How long the scope move and the activation call may take together:
/// past it the app keeps running where it started (or detached, when
/// there was no bus to begin with) and the panel still closes.
pub const SCOPE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long a spawn error stays drawn in the panel before it clears.
pub const ERROR_HOLD: Duration = Duration::from_secs(2);

const ACTIVATION_TOKEN_ENV: &str = "XDG_ACTIVATION_TOKEN";
const STARTUP_ID_ENV: &str = "DESKTOP_STARTUP_ID";
/// The marker environment naming the detached helper's re-exec: the
/// parent sets it, the helper unsets it before the app starts, so the
/// app never sees it and a helper never re-execs itself.
const DETACH_ENV: &str = "SCOOTLAUNCH_DETACH";
/// The slice systemd's desktop-environment page holds normal
/// applications in (`app.slice`).
const APP_SLICE: &str = "app.slice";
/// At most this long an id survives into a scope name: the whole unit
/// stays far under systemd's 256-byte limit.
const MAX_SCOPE_ID: usize = 96;
/// Eight hex characters of randomness per scope name.
const RANDOM_BYTES: usize = 4;

/// An activation token: a bearer credential, so `Debug` never prints
/// it (pinned by the test below).
pub struct Token(String);

impl Token {
    pub fn new(token: String) -> Self {
        Token(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// What the panel passes per launch: resolved without touching the
/// process environment, so tests build it literally.
pub struct Ctx {
    /// `$XDG_STATE_HOME` (the frecency file's parent), if nameable.
    pub state_home: Option<PathBuf>,
    /// `$HOME`, for the working-directory fallback.
    pub home: Option<PathBuf>,
    /// Whether `DISPLAY` is set (the X startup id rides the token).
    pub display_set: bool,
    /// Epoch seconds now, for the frecency record.
    pub now: u64,
}

/// Launches an indexed entry (or one of its actions): activation for
/// a `DBusActivatable` entry with an `Exec` fallback, else `Exec`.
/// Silent on success (the panel just closes): `Ok(())` covers a fresh
/// spawn in its scope, an in-place activation, a detached spawn, and a
/// spawn whose scope move was refused (one warning, still running).
/// Records frecency on the way out. Answers `Err` (nothing recorded,
/// nothing started) only when nothing could start.
pub fn launch_entry(
    entry: &Entry,
    action: Option<&Action>,
    token: Option<&Token>,
    ctx: &Ctx,
) -> Result<(), String> {
    let argv = match action {
        Some(action) => &action.exec,
        None => &entry.exec,
    };
    if entry.dbus_activatable {
        match activate_entry(entry, action, token) {
            Ok(()) => {
                record(ctx, &entry.id);
                return Ok(());
            }
            Err(reason) => {
                if argv.is_empty() {
                    return Err(reason);
                }
                warn(format_args!("scootlaunch: {reason}, falling back to Exec"));
            }
        }
    }
    if argv.is_empty() {
        return Err("nothing to launch: no Exec and no activation".to_string());
    }
    let key = match action {
        Some(action) => format!("{}#{}", entry.id, action.id),
        None => entry.id.clone(),
    };
    launch_exec(&key, &entry.id, argv, entry.workdir.as_deref(), token, ctx)
}

/// Launches an ad-hoc command (the panel's `Ctrl+Enter` on typed
/// text): `argv` split on spaces by the caller. Frecency keys it by
/// `run:<argv0>`, as the frecency file's docs say.
pub fn launch_command(argv: &[String], token: Option<&Token>, ctx: &Ctx) -> Result<(), String> {
    let Some(first) = argv.first().filter(|arg| !arg.is_empty()) else {
        return Err("nothing to run: the text is empty".to_string());
    };
    let key = format!("run:{first}");
    launch_exec(&key, &command_scope_base(first), argv, None, token, ctx)
}

/// The shared `Exec` path: direct spawn plus the scope move when a
/// bus answers, the detached spawn when none does. `id` names the
/// desktop entry (the scope and the frecency record); `scope_base`
/// is what the scope derives from when there is no entry (an ad-hoc
/// command's `argv[0]`).
fn launch_exec(
    frecency_key: &str,
    scope_base: &str,
    argv: &[String],
    workdir: Option<&str>,
    token: Option<&Token>,
    ctx: &Ctx,
) -> Result<(), String> {
    let cwd = resolve_cwd(workdir, ctx.home.as_deref());
    let display_set = ctx.display_set;
    if bus_dials() {
        let pid = spawn_direct(argv, cwd.as_deref(), token, display_set)?;
        let scope = scope_name(scope_base, &random_suffix());
        let description = format!("scootlaunch {scope_base}");
        let deadline = Instant::now() + SCOPE_TIMEOUT;
        match start_transient_unit(&scope, pid, &description, deadline) {
            Ok(()) => {
                record(ctx, frecency_key);
                Ok(())
            }
            Err(reason) => {
                warn(format_args!("scootlaunch: {reason}; the app keeps running"));
                record(ctx, frecency_key);
                Ok(())
            }
        }
    } else {
        spawn_detached(argv, cwd.as_deref(), token, display_set)?;
        record(ctx, frecency_key);
        Ok(())
    }
}

/// Whether a session bus answers a dial at all (no handshake: a fast
/// probe before deciding direct against detached).
fn bus_dials() -> bool {
    let Ok(addr) = bus_addr() else {
        return false;
    };
    dial(&addr).is_ok()
}

/// The `DBusActivatable` call for an entry: `ActivateAction` for a
/// desktop action, `Activate` for the entry itself. The bound is the
/// scope one: a slow bus must not hold the panel past it either.
fn activate_entry(
    entry: &Entry,
    action: Option<&Action>,
    token: Option<&Token>,
) -> Result<(), String> {
    let Some(bus) = dbus_bus_name(&entry.id) else {
        return Err(format!(
            "cannot activate {}: its bus name is not a bus name",
            short(&entry.id)
        ));
    };
    let Some(path) = dbus_object_path(&bus) else {
        return Err(format!(
            "cannot activate {}: its object path is not a path",
            short(&entry.id)
        ));
    };
    let token = token.filter(|token| !token.is_empty()).map(Token::as_str);
    let deadline = Instant::now() + SCOPE_TIMEOUT;
    match action {
        Some(action) => app_activate_action(&bus, &path, &action.id, token, deadline),
        None => app_activate(&bus, &path, token, deadline),
    }
}

fn record(ctx: &Ctx, key: &str) {
    let mut history = load(ctx.state_home.as_deref());
    history.record(ctx.state_home.as_deref(), key, ctx.now);
}

/// The app id for `set_app_id` and human lines: the desktop id
/// without its `.desktop` suffix.
pub fn app_id_for(id: &str) -> &str {
    id.strip_suffix(".desktop").unwrap_or(id)
}

/// The bus name for a `DBusActivatable` entry: the id without
/// `.desktop`. `None` when that is not a valid bus name (then `Exec`
/// is the only path).
pub fn dbus_bus_name(id: &str) -> Option<String> {
    let bus = app_id_for(id);
    if bus.is_empty() {
        return None;
    }
    check_name(bus).ok().map(|_| bus.to_string())
}

/// The object path for a bus name: a leading `/`, dots to slashes,
/// dashes to underscores. `None` when that is not a valid path.
pub fn dbus_object_path(bus: &str) -> Option<String> {
    if bus.is_empty() {
        return None;
    }
    let mut path = String::with_capacity(bus.len() + 1);
    path.push('/');
    for char in bus.chars() {
        match char {
            '.' => path.push('/'),
            '-' => path.push('_'),
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' => path.push(char),
            _ => return None,
        }
    }
    if path.len() < 2 || path.contains("//") || path.len() > 512 {
        return None;
    }
    Some(path)
}

/// Escapes an id for a unit name: what systemd allows rides through,
/// everything else is `_`, overlong ids are cut at a char boundary.
pub fn escape_unit_id(id: &str) -> String {
    let mut out = String::new();
    for char in id.chars() {
        if out.len() >= MAX_SCOPE_ID {
            break;
        }
        match char {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' | '_' => out.push(char),
            _ => out.push('_'),
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// `app-scootlaunch-<escaped>-<random>.scope`: the scheme systemd's
/// desktop-environment page asks launchers for.
pub fn scope_name(id: &str, random: &str) -> String {
    format!("app-scootlaunch-{}-{random}.scope", escape_unit_id(id))
}

/// The scope base for an ad-hoc command: `basename(argv[0])`, with
/// `-` mapped to `_` (a generated name must not contain `-`, so it
/// cannot read as a launcher separator).
pub fn command_scope_base(argv0: &str) -> String {
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    let base = if base.is_empty() { argv0 } else { base };
    let mut out = String::new();
    for char in base.chars() {
        if out.len() >= MAX_SCOPE_ID {
            break;
        }
        match char {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' => out.push(char),
            '-' => out.push('_'),
            _ => out.push('_'),
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Eight hex characters from `/dev/urandom`; the pid and the clock
/// when there is no urandom to read (never empty, never a panic).
pub fn random_suffix() -> String {
    let mut bytes = [0u8; RANDOM_BYTES];
    if read_urandom(&mut bytes) {
        hex_of(&bytes)
    } else {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(0);
        let mut fallback = format!("{:x}{:x}", std::process::id(), nanos);
        while fallback.len() < RANDOM_BYTES * 2 {
            fallback.push('0');
        }
        fallback.truncate(RANDOM_BYTES * 2);
        fallback
    }
}

fn read_urandom(out: &mut [u8]) -> bool {
    let Ok(mut file) = std::fs::File::open("/dev/urandom") else {
        return false;
    };
    file.read_exact(out).is_ok()
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The working directory for a spawn: `Path=` when it names an
/// absolute directory that exists, else `$HOME` when that is a
/// directory, else inherited (`None`).
pub fn resolve_cwd(workdir: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = workdir.filter(|dir| !dir.is_empty()) {
        let path = Path::new(dir);
        if path.is_absolute() && path.is_dir() {
            return Some(path.to_path_buf());
        }
    }
    if let Some(home) = home
        && home.is_absolute()
        && home.is_dir()
    {
        return Some(home.to_path_buf());
    }
    None
}

/// Splits run text into an argv on spaces, dropping empties: what
/// the panel's `Ctrl+Enter` runs. An all-spaces text answers empty,
/// and the launch refuses that loudly.
pub fn split_run_argv(text: &str) -> Vec<String> {
    text.split(' ')
        .filter(|piece| !piece.is_empty())
        .map(str::to_string)
        .collect()
}

/// A hostile argv[0] in a human line: the first 128 chars, never the
/// whole megabyte.
fn short(program: &str) -> String {
    let mut out = String::new();
    for char in program.chars().take(128) {
        out.push(char);
    }
    if program.chars().count() > 128 {
        out.push('…');
    }
    out
}

// --- Spawning ---------------------------------------------------------

/// The environment delta for a spawn: the token carriers to remove
/// (always — a consumed token must not leak), then the fresh token to
/// set (as `XDG_ACTIVATION_TOKEN`, and as `DESKTOP_STARTUP_ID` while
/// `DISPLAY` is set, for X clients). Pure, so the rules pin in a unit
/// test; every spawn path applies the same delta.
pub fn child_env_delta(
    token: Option<&str>,
    display_set: bool,
) -> (Vec<&'static str>, Vec<(&'static str, String)>) {
    let remove = vec![ACTIVATION_TOKEN_ENV, STARTUP_ID_ENV];
    let mut set = Vec::new();
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        set.push((ACTIVATION_TOKEN_ENV, token.to_string()));
        if display_set {
            set.push((STARTUP_ID_ENV, token.to_string()));
        }
    }
    (remove, set)
}

fn apply_child_env(command: &mut Command, token: Option<&Token>, display_set: bool) {
    let (remove, set) = child_env_delta(token.map(Token::as_str), display_set);
    for key in remove {
        command.env_remove(key);
    }
    for (key, value) in set {
        command.env(key, value);
    }
}

/// Starts `argv` directly: the environment rules, the working
/// directory, stdin from `/dev/null`. Answers the child's pid; the
/// `Child` handle is dropped (the launcher exits milliseconds later
/// and init reaps the child, which long outlives it).
fn spawn_direct(
    argv: &[String],
    cwd: Option<&Path>,
    token: Option<&Token>,
    display_set: bool,
) -> Result<u32, String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("nothing to launch".to_string());
    };
    if program.is_empty() {
        return Err("nothing to launch".to_string());
    }
    let mut command = Command::new(program);
    command.args(args);
    command.stdin(Stdio::null());
    apply_child_env(&mut command, token, display_set);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    match command.spawn() {
        Ok(child) => Ok(child.id()),
        Err(error) => Err(format!("cannot start {}: {error}", short(program))),
    }
}

/// The detached re-exec spec: everything the helper needs, over
/// stdin as one JSON object (no new CLI flag, nothing on the panel's
/// command line).
#[derive(serde::Serialize, serde::Deserialize)]
struct DetachSpec {
    argv: Vec<String>,
    cwd: Option<String>,
    remove: Vec<String>,
    set: Vec<(String, String)>,
}

/// Starts `argv` detached: our own binary re-executed as a helper
/// (over stdin, under [`DETACH_ENV`]), which calls `setsid`, starts
/// the app, and exits — the double fork with `setsid`, with no
/// `unsafe` and no helper binary. The grandchild's pid is unknowable
/// here; the scope move is skipped (there is no bus to move it with).
fn spawn_detached(
    argv: &[String],
    cwd: Option<&Path>,
    token: Option<&Token>,
    display_set: bool,
) -> Result<(), String> {
    if argv.first().is_none_or(|arg| arg.is_empty()) {
        return Err("nothing to launch".to_string());
    }
    let exe = std::env::current_exe().map_err(|error| format!("cannot detach: {error}"))?;
    let (remove, set) = child_env_delta(token.map(Token::as_str), display_set);
    let spec = DetachSpec {
        argv: argv.to_vec(),
        cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
        remove: remove.iter().map(|key| key.to_string()).collect(),
        set: set
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    };
    let json = serde_json::to_string(&spec).map_err(|error| format!("cannot detach: {error}"))?;
    if json.len() > 256 * 1024 {
        return Err("cannot detach: the command is too large".to_string());
    }
    let mut command = Command::new(exe);
    command.env(DETACH_ENV, "1");
    command.stdin(Stdio::piped());
    command.stdout(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot detach: {error}"))?;
    let wrote = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(json.as_bytes()).is_ok(),
        None => false,
    };
    if !wrote {
        return Err("cannot detach: the helper went away".to_string());
    }
    // The helper fails fast when there is nothing to start (a missing
    // target fails its `exec` in milliseconds): wait up to 200 ms for
    // that verdict, so a vanished target still draws its error instead
    // of exiting 0. Past the bound the helper is assumed launched —
    // the panel must not linger on it — and init reaps it whenever it
    // exits.
    match wait_helper(&mut child, Duration::from_millis(200)) {
        HelperDone::Launched => Ok(()),
        HelperDone::Failed => Err(format!(
            "cannot start {}: the detached launch failed (see stderr)",
            short(argv.first().map(String::as_str).unwrap_or(""))
        )),
    }
}

/// What the helper wait ended with.
enum HelperDone {
    Launched,
    Failed,
}

/// Waits up to `bound` for the helper to exit: `Failed` for a nonzero
/// status, `Launched` for zero or for still running past the bound.
fn wait_helper(child: &mut std::process::Child, bound: Duration) -> HelperDone {
    let deadline = Instant::now() + bound;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    HelperDone::Launched
                } else {
                    HelperDone::Failed
                };
            }
            Ok(None) => {}
            Err(_) => return HelperDone::Launched,
        }
        if Instant::now() >= deadline {
            return HelperDone::Launched;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The helper half of [`spawn_detached`]: reads the spec, leaves the
/// launcher's session and process group, starts the app, exits. `0`
/// when the app started, `1` otherwise.
pub fn detach_main() -> i32 {
    match detach_run() {
        Ok(()) => 0,
        Err(reason) => {
            let _ = std::io::stderr().write_fmt(format_args!("scootlaunch: {reason}\n"));
            1
        }
    }
}

fn detach_run() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_end(&mut input)
        .map_err(|error| format!("the detach helper cannot read its spec: {error}"))?;
    let spec: DetachSpec = serde_json::from_slice(&input)
        .map_err(|error| format!("the detach helper got a bad spec: {error}"))?;
    let Some((program, args)) = spec.argv.split_first() else {
        return Err("the detach helper got nothing to launch".to_string());
    };
    if program.is_empty() {
        return Err("the detach helper got nothing to launch".to_string());
    }
    // Out of the launcher's session and process group; already a
    // session leader (or otherwise out of reach of `setsid`) is not a
    // failure — the reparenting below still detaches the app.
    let _ = rustix::process::setsid();
    let mut command = Command::new(program);
    command.args(args);
    command.stdin(Stdio::null());
    command.env_remove(DETACH_ENV);
    for key in &spec.remove {
        command.env_remove(key);
    }
    for (key, value) in &spec.set {
        if key == DETACH_ENV {
            continue;
        }
        command.env(key, value);
    }
    if let Some(dir) = spec.cwd.filter(|dir| !dir.is_empty()) {
        command.current_dir(dir);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("cannot start {}: {error}", short(program)))
}

/// Whether this process is the detached helper's re-exec.
pub fn is_detach_helper() -> bool {
    std::env::var_os(DETACH_ENV).is_some_and(|value| value == "1")
}

// --- The bus calls ----------------------------------------------------

/// Moves `pid` into `scope` (`Slice`, `PIDs`, `CollectMode`, a
/// `Description`): `StartTransientUnit` with mode `fail`. Answers the
/// job path; any refusal or timeout is an `Err` reason for the
/// caller to log once (never the token: none of these strings carries
/// one).
pub fn start_transient_unit(
    scope: &str,
    pid: u32,
    description: &str,
    deadline: Instant,
) -> Result<(), String> {
    let Some((sig, body)) = transient_body(scope, pid, description) else {
        return Err("cannot move the app into its scope: the call does not fit".to_string());
    };
    let addr = bus_addr().map_err(|()| {
        "no session bus: set DBUS_SESSION_BUS_ADDRESS or XDG_RUNTIME_DIR".to_string()
    })?;
    let call = Call {
        dest: "org.freedesktop.systemd1",
        path: "/org/freedesktop/systemd1",
        interface: "org.freedesktop.systemd1.Manager",
        member: "StartTransientUnit",
        sig: &sig,
        body: &body,
    };
    let (reply_sig, reply_body) = dbus_call_at(&addr, &call, deadline)?;
    if reply_sig != "o" {
        return Err("the scope move answered out of shape".to_string());
    }
    let mut reader = scoot_dbus::proto::Reader::le(&reply_body);
    match reader.str() {
        Ok(_) => Ok(()),
        Err(()) => Err("the scope move answered out of shape".to_string()),
    }
}

/// The `StartTransientUnit` body: `ssa(sv)a(sa(sv))` — the name, the
/// mode, the properties, the empty aux. `None` when it overflows
/// (a hostile id is escaped first, so this is unreachable in
/// practice, but never a panic).
pub fn transient_body(scope: &str, pid: u32, description: &str) -> Option<(String, Vec<u8>)> {
    let mut out = Writer::new();
    out.str(scope);
    out.str("fail");
    let properties = out.open_array(8)?;
    if out.open_struct() {
        out.str("Slice");
        out.variant("s");
        out.str(APP_SLICE);
        out.close_struct();
    }
    if out.open_struct() {
        out.str("PIDs");
        out.variant("au");
        let pids = out.open_array(4)?;
        out.u32(pid);
        out.close_array(pids);
        out.close_struct();
    }
    if out.open_struct() {
        out.str("CollectMode");
        out.variant("s");
        out.str("inactive-or-failed");
        out.close_struct();
    }
    if out.open_struct() {
        out.str("Description");
        out.variant("s");
        out.str(description);
        out.close_struct();
    }
    out.close_array(properties);
    let aux = out.open_array(8)?;
    out.close_array(aux);
    let body = out.take_body()?;
    Some(("ssa(sv)a(sa(sv))".to_string(), body))
}

/// `org.freedesktop.Application.Activate` with `activation-token` in
/// `platform_data` (empty when there is no token to carry).
pub fn app_activate(
    bus: &str,
    path: &str,
    token: Option<&str>,
    deadline: Instant,
) -> Result<(), String> {
    let Some((sig, body)) = activate_body(token) else {
        return Err("cannot activate: the call does not fit".to_string());
    };
    let addr = bus_addr().map_err(|()| {
        "no session bus: set DBUS_SESSION_BUS_ADDRESS or XDG_RUNTIME_DIR".to_string()
    })?;
    let call = Call {
        dest: bus,
        path,
        interface: "org.freedesktop.Application",
        member: "Activate",
        sig: &sig,
        body: &body,
    };
    let (reply_sig, _) = dbus_call_at(&addr, &call, deadline)?;
    if reply_sig.is_empty() {
        Ok(())
    } else {
        Err("activation answered out of shape".to_string())
    }
}

/// `ActivateAction` for a desktop action: the action name, no
/// parameter, and the same `platform_data` as [`app_activate`].
pub fn app_activate_action(
    bus: &str,
    path: &str,
    action: &str,
    token: Option<&str>,
    deadline: Instant,
) -> Result<(), String> {
    let Some((sig, body)) = activate_action_body(action, token) else {
        return Err("cannot activate: the call does not fit".to_string());
    };
    let addr = bus_addr().map_err(|()| {
        "no session bus: set DBUS_SESSION_BUS_ADDRESS or XDG_RUNTIME_DIR".to_string()
    })?;
    let call = Call {
        dest: bus,
        path,
        interface: "org.freedesktop.Application",
        member: "ActivateAction",
        sig: &sig,
        body: &body,
    };
    let (reply_sig, _) = dbus_call_at(&addr, &call, deadline)?;
    if reply_sig.is_empty() {
        Ok(())
    } else {
        Err("activation answered out of shape".to_string())
    }
}

/// The `Activate` body: `a{sv}` with one entry (`activation-token`)
/// or none. `None` on overflow.
pub fn activate_body(token: Option<&str>) -> Option<(String, Vec<u8>)> {
    let mut out = Writer::new();
    platform_data(&mut out, token)?;
    let body = out.take_body()?;
    Some(("a{sv}".to_string(), body))
}

/// The `ActivateAction` body: `sava{sv}` — the action, an empty
/// parameter, the platform data. `None` on overflow.
pub fn activate_action_body(action: &str, token: Option<&str>) -> Option<(String, Vec<u8>)> {
    let mut out = Writer::new();
    out.str(action);
    let params = out.open_array(1)?;
    out.close_array(params);
    platform_data(&mut out, token)?;
    let body = out.take_body()?;
    Some(("sava{sv}".to_string(), body))
}

fn platform_data(out: &mut Writer, token: Option<&str>) -> Option<()> {
    let dict = out.open_array(8)?;
    if let Some(token) = token
        && out.open_struct()
    {
        out.str("activation-token");
        out.variant("s");
        out.str(token);
        out.close_struct();
    }
    out.close_array(dict);
    Some(())
}

/// One method call: its destination, object, interface, member and
/// pre-marshalled body, borrowed.
pub struct Call<'a> {
    pub dest: &'a str,
    pub path: &'a str,
    pub interface: &'a str,
    pub member: &'a str,
    pub sig: &'a str,
    pub body: &'a [u8],
}

/// One method call against `addr` with a total deadline: dial, the
/// SASL handshake, `Hello`, the call, its reply. Every read and write
/// is bounded by what is left of `deadline`; past it every path
/// answers `Err`, never a hang. Answers the reply's signature and
/// body; an error reply is an `Err` with the error's name.
pub fn dbus_call_at(
    addr: &BusAddr,
    call: &Call<'_>,
    deadline: Instant,
) -> Result<(String, Vec<u8>), String> {
    let mut stream = dial(addr)?;
    handshake(&mut stream, deadline)?;
    hello(&mut stream, deadline)?;
    let mut frame = Writer::new();
    frame.begin_call(
        2,
        call.dest,
        call.path,
        call.interface,
        call.member,
        call.sig,
        0,
    );
    frame.raw(call.body);
    let bytes = frame
        .finish()
        .ok_or_else(|| "the call does not fit".to_string())?;
    write_all(&mut stream, &bytes, deadline)?;
    let mut staged = Vec::new();
    loop {
        let frame = read_frame(&mut stream, &mut staged, deadline)?;
        let message = Message::parse(&frame).map_err(|()| "a bad bus frame".to_string())?;
        match message.kind {
            Kind::MethodReturn if message.reply_serial == Some(2) => {
                return Ok((message.signature.to_string(), message.body.rest().to_vec()));
            }
            Kind::Error if message.reply_serial == Some(2) => {
                return Err(message
                    .error
                    .unwrap_or("org.freedesktop.DBus.Error.Failed")
                    .to_string());
            }
            _ => {}
        }
    }
}

fn dial(addr: &BusAddr) -> Result<UnixStream, String> {
    match addr {
        BusAddr::Path(path) => UnixStream::connect(path).map_err(|error| {
            format!(
                "cannot reach the session bus at {}: {error}",
                path.display()
            )
        }),
        BusAddr::Abstract(name) => {
            use std::os::linux::net::SocketAddrExt as _;
            let sock = std::os::unix::net::SocketAddr::from_abstract_name(name)
                .map_err(|_| "cannot reach the session bus: a bad abstract name".to_string())?;
            UnixStream::connect_addr(&sock)
                .map_err(|error| format!("cannot reach the session bus: {error}"))
        }
    }
}

fn left(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|span| !span.is_zero())
        .ok_or_else(|| "the bus did not answer in time".to_string())
}

fn bound(stream: &UnixStream, deadline: Instant) -> Result<(), String> {
    let span = left(deadline)?;
    stream
        .set_read_timeout(Some(span))
        .map_err(|error| format!("cannot bound the bus: {error}"))?;
    stream
        .set_write_timeout(Some(span))
        .map_err(|error| format!("cannot bound the bus: {error}"))?;
    Ok(())
}

fn write_all(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> Result<(), String> {
    while !bytes.is_empty() {
        bound(stream, deadline)?;
        match stream.write(bytes) {
            Ok(0) => return Err("the bus closed under a write".to_string()),
            // `n` is at most `bytes.len()` by `Write`'s contract; the
            // clamp keeps the slicing below total even so.
            Ok(n) => bytes = &bytes[n.min(bytes.len())..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(bus_io_error(error)),
        }
    }
    Ok(())
}

fn bus_io_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            "the bus did not answer in time".to_string()
        }
        _ => format!("bus I/O failed: {error}"),
    }
}

/// The bus's next line (to `\r\n`), at most 512 bytes: the SASL
/// handshake's shape. A peer that never finishes a line fails the
/// call instead of hanging it.
fn read_line(stream: &mut UnixStream, deadline: Instant) -> Result<String, String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        bound(stream, deadline)?;
        match stream.read(&mut byte) {
            Ok(0) => return Err("the bus went quiet".to_string()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(bus_io_error(error)),
        }
        line.push(byte[0]);
        if line.len() > 512 {
            return Err("a set-up line too long".to_string());
        }
        if line.len() >= 2 && line[line.len() - 2] == b'\r' && line[line.len() - 1] == b'\n' {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).map_err(|_| "a bad set-up line".to_string());
        }
    }
}

/// Dial, authenticate (`EXTERNAL`, the empty initial response — the
/// exact bytes `busctl` sends) and say `Hello`: the shaped-down twin
/// of the shared client's set-up, with the caller's deadline instead
/// of its own.
fn handshake(stream: &mut UnixStream, deadline: Instant) -> Result<(), String> {
    write_all(stream, &[0], deadline)?;
    write_all(stream, b"AUTH EXTERNAL\r\n", deadline)?;
    let line = read_line(stream, deadline)?;
    if line != "DATA" && !line.starts_with("DATA ") {
        return Err("the bus answered out of turn".to_string());
    }
    write_all(stream, b"DATA\r\n", deadline)?;
    let line = read_line(stream, deadline)?;
    if !line.starts_with("OK ") {
        return Err("no OK to EXTERNAL".to_string());
    }
    write_all(stream, b"BEGIN\r\n", deadline)?;
    Ok(())
}

fn hello(stream: &mut UnixStream, deadline: Instant) -> Result<(), String> {
    let mut call = Writer::new();
    call.begin_call(1, BUS_NAME, BUS_PATH, BUS_INTERFACE, "Hello", "", 0);
    let bytes = call
        .finish()
        .ok_or_else(|| "the Hello call does not fit".to_string())?;
    write_all(stream, &bytes, deadline)?;
    let mut staged = Vec::new();
    loop {
        let frame = read_frame(stream, &mut staged, deadline)?;
        let message = Message::parse(&frame).map_err(|()| "a bad Hello reply".to_string())?;
        match message.kind {
            Kind::MethodReturn if message.reply_serial == Some(1) => {
                if message.signature != "s" {
                    return Err("Hello answered out of shape".to_string());
                }
                return Ok(());
            }
            Kind::Error if message.reply_serial == Some(1) => {
                return Err("the bus errored Hello".to_string());
            }
            _ => {}
        }
    }
}

/// The next whole frame off the bus, staged across reads: at most
/// one capped message plus a read's worth staged, so a flood backs
/// up in the socket instead of this allocation.
fn read_frame(
    stream: &mut UnixStream,
    staged: &mut Vec<u8>,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    loop {
        match frame_at(staged) {
            Ok(Some(len)) => return Ok(staged.drain(..len).collect()),
            Ok(None) => {}
            Err(()) => return Err("a bad bus frame".to_string()),
        }
        if staged.len() > scoot_dbus::proto::MAX_MESSAGE + 64 * 1024 {
            return Err("the set-up never framed".to_string());
        }
        bound(stream, deadline)?;
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return Err("the bus went away".to_string()),
            Ok(n) => staged.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(bus_io_error(error)),
        }
    }
}
