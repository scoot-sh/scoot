//! IPC look swaps: `Request::LookApply` runs a swap in a child and answers
//! when it exits; `Request::Looks` answers synchronously.
//!
//! A swap that only touches local files cannot be reached from a client
//! that has no files -- a Mac driving a VM's compositor over the socket --
//! and `Action(Spawn "scoot look apply …")` answers before the swap and
//! never says whether it refused. So the compositor runs the swap itself:
//! it validates the name (never a path), spawns `scoot look apply NAME
//! --from-compositor` as a child with the session environment (the scootbg
//! run's pattern -- see `wallpaper.rs` -- including this session's own
//! `SCOOT_LOOK_STATE`, so a headless session's swap never touches the user
//! state), and answers when the child exits, off the loop like `WaitIdle`
//! (see `connection.rs`): the requesting connection parks in
//! `PendingLookApply` while every other connection keeps being served --
//! which is also how the child's own `ReloadLook` gets answered mid-swap.
//! One swap at a time; a second is refused with an error.
//!
//! The child's `--json` document on its stdout is the reply's source: the
//! compositor parses the first JSON value (a grandchild holding the pipe
//! open cannot wedge this -- the pipe is non-blocking and the read is
//! capped) and shapes `Response::LookApplied` from it, or an `Error` from
//! its failure shape. Nothing here renders: rendering, font resolution
//! and the image check run in the child's process, never on the loop
//! thread.
//!
//! A timed preview (`for_ms`) arms a calloop timer at a successful swap:
//! when it fires, the recorded choice is read back, and only if its
//! generation still holds (no newer choice) does the previous look swap
//! back -- the same generation rule `apply --for` keeps client-side. A
//! swap still running when the timer fires does not drop the revert: it
//! waits stashed until the swap finishes (or times out), then re-checks,
//! so the preview never sticks past its delay with no timer left.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use scoot_ipc::{LookEntry, Response, encode};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use super::outbound::Outbound;
use super::slots::Slot;
use crate::compositor::State;

#[cfg(test)]
mod tests;

/// How long the apply child may run before the parked request is answered
/// with an error anyway.
///
/// The child holds the state-dir lock and drives one 30-second-bounded
/// reload per app, so a healthy swap with a few bars lands in well under a
/// minute; past this the requester is told the swap timed out rather than
/// parked forever. The child itself is never killed for it -- killing a
/// swap mid-repoint trades a slow swap for a half-moved one -- it keeps
/// running, its exit is still reaped, and the outcome lands in the log.
const LOOK_APPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a parked reply is pushed at a client that stopped reading
/// before it is dropped. Look replies are a few hundred bytes -- a peer
/// that will not take them is gone, not slow.
const LOOK_REPLY_STALL: Duration = Duration::from_secs(10);

/// The longest apply-child reply read. The `--json` document is a few
/// hundred bytes of names and key lists; this only bounds a child gone
/// wrong (or a grandchild chatting on the same pipe).
const MAX_CHILD_OUTPUT: usize = 1024 * 1024;

/// A `look_apply` request parked while its child runs -- or while its
/// reply drains to a slow client.
pub(crate) struct PendingLookApply {
    /// The apply child's pid, for the reaper. `None` once reaped.
    pid: Option<u32>,
    /// The child's piped stdout, non-blocking. Read once, at the reap.
    stdout: Option<std::process::ChildStdout>,
    /// The look asked for, for logs and replies.
    name: String,
    /// A timed preview's hold in milliseconds, for the revert timer.
    /// `None` means stay.
    for_ms: Option<u64>,
    /// The look active before this swap, for the revert. Read before the
    /// spawn; `None` when `for_ms` is unset (no revert to schedule).
    previous: Option<String>,
    /// When the child was spawned. Bounds the run (see
    /// [`LOOK_APPLY_TIMEOUT`]).
    started: Instant,
    /// Whether the child's exit has been turned into a queued reply.
    /// `false` until the reap; afterwards the entry only drains.
    finished: bool,
    /// When a byte of the reply last went out -- or, if none has, when it
    /// was queued. Bounds the drain (see [`LOOK_REPLY_STALL`]).
    last_progress: Instant,
    /// The parked connection, taken like `WaitIdle`'s: its socket (sharing
    /// the connection's non-blocking flags), its unsent queue ahead of the
    /// reply, and its connection-table slot. `None` for a timer revert,
    /// which answers nobody.
    client: Option<ParkedClient>,
    /// The generation guard a timer revert's child carries, with its
    /// remaining lock-contention retries. `Some` only for timer reverts
    /// (`client` is `None` there too): the child refuses as superseded
    /// rather than undoing a newer choice that landed after the guard was
    /// read, and a child that fails on a still-held lock re-arms through
    /// [`DeferredLookRevert`] instead of dropping the revert.
    revert_guard: Option<RevertGuard>,
}

/// The generation a timer revert restores only while the choice still
/// holds, and how many of its lock-contention retries remain.
#[derive(Clone, Copy)]
pub(crate) struct RevertGuard {
    generation: u64,
    retries: u8,
}

/// A timed preview's revert that fired while a swap was running: it waits
/// for the running swap to finish (or time out) instead of being dropped,
/// then re-reads the generation and reverts only if the choice has not
/// moved. A newer preview's timer overwrites an older one's: the older
/// generation can no longer hold once a newer preview's swap completed,
/// so its revert would stand down anyway.
#[derive(Clone)]
pub(crate) struct DeferredLookRevert {
    previous: String,
    generation: u64,
    state_dir: PathBuf,
    retries: u8,
}

/// How many lock-contention rounds a deferred revert waits out before
/// giving up loudly: each round is one spawned child waiting out the
/// holder, so eight rounds ride out a swap whose apps hang their bounded
/// reloads. Every round re-checks the generation, so a newer choice still
/// stands the revert down instead of being undone.
const DEFERRED_REVERT_RETRIES: u8 = 8;

struct ParkedClient {
    stream: UnixStream,
    outbound: Outbound,
    _slot: Option<Slot>,
}

/// An apply child spawned and ready to park: everything
/// `check_and_spawn_look_apply` produces before the connection's queue
/// and slot join it in `start_look_apply`.
struct SpawnedLookApply {
    pid: u32,
    stdout: std::process::ChildStdout,
    name: String,
    for_ms: Option<u64>,
    previous: Option<String>,
}

/// What `start_look_apply` refused before (or without) spawning: the
/// reply, plus the connection's queue and slot back. The queue and slot
/// are taken up front (the park needs them on success); dropping them on
/// a refusal would lose replies already queued and leak connection-table
/// capacity for the rest of the session, so they ride back out and the
/// caller puts them back before answering.
pub(super) struct LookApplyRefused {
    pub(super) response: Response,
    pub(super) outbound: Outbound,
    pub(super) slot: Option<Slot>,
}

/// How the apply child ended, from its `waitpid` status.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ChildEnd {
    Success,
    Code(i32),
    Signal(i32),
    /// Reaped elsewhere before this ran: gone without a status.
    Unknown,
}

impl ChildEnd {
    fn from_waited(waited: super::super::wallpaper::Waited) -> Self {
        match waited {
            super::super::wallpaper::Waited::Running => ChildEnd::Unknown,
            super::super::wallpaper::Waited::Gone => ChildEnd::Unknown,
            super::super::wallpaper::Waited::Exited(status) => {
                if libc::WIFEXITED(status) {
                    match libc::WEXITSTATUS(status) {
                        0 => ChildEnd::Success,
                        code => ChildEnd::Code(code),
                    }
                } else if libc::WIFSIGNALED(status) {
                    ChildEnd::Signal(libc::WTERMSIG(status))
                } else {
                    ChildEnd::Unknown
                }
            }
        }
    }
}

/// What `start_look_apply` refused before (or without) spawning, answered
/// synchronously on the requesting connection.
fn refused(message: String) -> Response {
    Response::error(message)
}

/// Validates a look name for an IPC swap: `[a-z0-9-]{1,32}`, never a path.
/// The child re-checks (it never trusts this side), but refusing here
/// means a path never even spawns.
pub(super) fn valid_look_apply_name(name: &str) -> Result<(), String> {
    if scoot_look::grammar::is_name(name) {
        Ok(())
    } else {
        Err(format!(
            "refused: `{name}` is not a look name ([a-z0-9-]{{1,32}}); a path is never a look"
        ))
    }
}

/// Whether a timed preview reverts: only when the recorded choice still
/// holds the generation the swap wrote. A newer choice (any later apply,
/// from any channel) drops the revert -- a preview never undoes a newer
/// choice.
pub(super) fn should_revert(current_generation: Option<u64>, wanted: u64) -> bool {
    current_generation == Some(wanted)
}

/// Whether a revert child's failure is a busy state-dir lock (a swap
/// still running): worth waiting out, unlike a refusal, which a retry
/// would only repeat. Matches [`scoot_look::state::LOCK_CONTENDED`], the
/// same marker the CLI reverter matches on.
fn is_lock_contention(message: &str) -> bool {
    message.contains(scoot_look::state::LOCK_CONTENDED)
}

/// Reads the swapped generation out of a successful child's first JSON
/// value, for the preview timer's guard. The `--json` success document
/// carries it; any other shape (including every failure) yields nothing,
/// and no timer arms.
pub(super) fn child_generation(output: &[u8]) -> Option<u64> {
    first_json_value(output)?.get("generation")?.as_u64()
}

/// The first JSON value on the child's pipe, ignoring whatever follows.
/// Anything after the document is a grandchild's chatter, not the reply.
fn first_json_value(output: &[u8]) -> Option<serde_json::Value> {
    serde_json::Deserializer::from_slice(output)
        .into_iter::<serde_json::Value>()
        .next()
        .and_then(|parsed| parsed.ok())
}

/// Shapes `Response::LookApplied` (or the `Error` it degrades to) from the
/// apply child's end state and whatever its stdout carried.
///
/// The child's `--json` shapes come from `scoot look apply`: success is
/// `{"applied": NAME, "reloaded": [...], "shadowed": [...], ...}` and
/// failure `{"target", "applied": false, "error", "rolled_back_to",
/// "rollback_errors"}`.
pub(super) fn look_applied_from_child(name: &str, end: ChildEnd, output: &[u8]) -> Response {
    let report = first_json_value(output);
    // A zero exit with a failure document: believe the document, not the
    // code. (The tool exits nonzero on failure; this is belt and braces
    // for a child that got it wrong.)
    let end = match (end, &report) {
        (ChildEnd::Success, Some(serde_json::Value::Object(doc)))
            if doc.get("applied").and_then(|v| v.as_bool()) == Some(false) =>
        {
            ChildEnd::Unknown
        }
        _ => end,
    };
    match end {
        ChildEnd::Success => match report {
            Some(serde_json::Value::Object(report)) => {
                let applied = report.get("applied").and_then(|v| v.as_str());
                let strings = |key: &str| {
                    report.get(key).and_then(|v| v.as_array()).map(|list| {
                        list.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                };
                match (applied, strings("reloaded"), strings("shadowed")) {
                    (Some(applied), Some(reloaded), Some(shadowed)) => Response::LookApplied {
                        name: applied.to_owned(),
                        applied: reloaded,
                        refused: Vec::new(),
                        shadowed,
                    },
                    _ => refused(unreadable_success(name)),
                }
            }
            _ => refused(unreadable_success(name)),
        },
        _ => match report {
            Some(serde_json::Value::Object(report)) => {
                match report.get("error").and_then(|v| v.as_str()) {
                    Some(error) => {
                        let mut message = error.to_owned();
                        if let Some(previous) =
                            report.get("rolled_back_to").and_then(|v| v.as_str())
                        {
                            message.push_str(&format!("; rolled back to {previous}"));
                        }
                        if let Some(errors) =
                            report.get("rollback_errors").and_then(|v| v.as_array())
                            && !errors.is_empty()
                        {
                            let details = errors
                                .iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("; ");
                            message.push_str(&format!("; ROLLED BACK WITH ERRORS: {details}"));
                        }
                        refused(message)
                    }
                    None => refused(child_status_line(name, end)),
                }
            }
            _ => refused(child_status_line(name, end)),
        },
    }
}

/// The refusal when the child succeeded but its reply cannot be
/// described: the swap may or may not have landed, and the reply says
/// which uncertainty, not a false certainty.
fn unreadable_success(name: &str) -> String {
    format!(
        "the look swap finished but its reply was unreadable: refusing it rather than \
         reporting a swap that cannot be described (look `{name}` may or may not be active; \
         `scoot look current` says which)"
    )
}

/// The fallback refusal when the child failed without a readable failure
/// document: the end state is the whole story.
fn child_status_line(name: &str, end: ChildEnd) -> String {
    let tail = "`scoot look current` says which look is active";
    match end {
        ChildEnd::Success => unreadable_success(name),
        ChildEnd::Code(code) => format!(
            "`scoot look apply {name}` exited with status {code} with no readable reply; {tail}"
        ),
        ChildEnd::Signal(signal) => format!(
            "`scoot look apply {name}` died to signal {signal} with no readable reply; {tail}"
        ),
        ChildEnd::Unknown => {
            format!("`scoot look apply {name}` failed with no readable reply; {tail}")
        }
    }
}

/// Reads the child's piped stdout to EOF: non-blocking, capped, never
/// hanging the loop thread on a grandchild that inherited the pipe and
/// outlives the child. A `WouldBlock` with the child reaped means no more
/// data is observable; whatever arrived parses or the reply is unreadable.
fn read_child_output(stdout: &mut std::process::ChildStdout) -> Vec<u8> {
    use std::io::Read as _;
    let mut out = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() >= MAX_CHILD_OUTPUT {
                    out.truncate(MAX_CHILD_OUTPUT);
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    out
}

impl State {
    /// The state directory this session's look swaps act on: the private
    /// dir for a private session, else the user dir resolved the way
    /// startup resolves it (an inherited `SCOOT_LOOK_STATE` ignored, so a
    /// nested session inside a private one still reports the user state).
    /// `None` only when no home resolves at all.
    pub(super) fn look_state_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.private_look_dir {
            return Some(dir.clone());
        }
        super::super::look_layer::state_dir(
            None,
            None,
            std::env::var_os("XDG_STATE_HOME"),
            std::env::var_os("HOME"),
        )
    }

    /// Serves `Request::Looks`: the catalog in search order, the active
    /// look's name, and the session's state directory. Synchronous -- one
    /// bounded directory scan and two small file reads, the same class of
    /// loop-thread I/O a reload already does.
    pub fn request_looks(&self) -> Response {
        let Some(state_dir) = self.look_state_dir() else {
            return refused(
                "no look state directory: neither XDG_STATE_HOME nor HOME is set".to_owned(),
            );
        };
        self.look_snapshot(&scootctl::look::catalog::discover(), &state_dir)
    }

    /// Shapes `Response::Looks` over an explicit catalog, so tests pin the
    /// shaping without touching the process environment.
    pub(super) fn look_snapshot(
        &self,
        catalog: &scootctl::look::catalog::Catalog,
        state_dir: &Path,
    ) -> Response {
        let _ = self;
        let looks = catalog
            .entries
            .iter()
            .map(|entry| LookEntry {
                name: entry.name.clone(),
                title: entry.raw.title.clone().unwrap_or_default(),
                polarity: entry
                    .raw
                    .polarity
                    .map(|polarity| polarity.as_str().to_owned())
                    .unwrap_or_else(|| "dark".to_owned()),
                origin: scootctl::look::catalog::origin_name(entry.origin).to_owned(),
                source: entry.dir.display().to_string(),
            })
            .collect();
        let active = match scoot_look::state::read_record(state_dir) {
            scoot_look::state::RecordRead::Present(record) => Some(record.name),
            // Missing or corrupt: no recorded choice to report. Corrupt is
            // loud elsewhere (startup repair, `scoot look current`); this
            // reply lists, it does not diagnose.
            scoot_look::state::RecordRead::Missing | scoot_look::state::RecordRead::Corrupt(_) => {
                None
            }
        };
        Response::Looks {
            looks,
            active,
            state_dir: state_dir.display().to_string(),
        }
    }

    /// Parks a `look_apply` request: validates, refuses a second concurrent
    /// swap, spawns the child, and holds the connection. `Ok(())` means
    /// parked (the caller closes the connection -- the reply comes later
    /// through the cloned socket); `Err` answers now, with the
    /// connection's queue and slot handed back (see [`LookApplyRefused`]).
    ///
    /// `pub(super)`, not `pub(crate)`: the parked slot it takes is only
    /// nameable inside `ipc`, which is exactly where its one caller
    /// (`connection.rs`) lives.
    pub(super) fn start_look_apply(
        &mut self,
        name: String,
        for_ms: Option<u64>,
        stream: UnixStream,
        outbound: Outbound,
        slot: Option<Slot>,
    ) -> Result<(), Box<LookApplyRefused>> {
        match self
            .check_and_spawn_look_apply(name, for_ms)
            .map_err(refused)
        {
            Ok(spawned) => {
                let now = Instant::now();
                self.pending_look = Some(PendingLookApply {
                    pid: Some(spawned.pid),
                    stdout: Some(spawned.stdout),
                    name: spawned.name,
                    for_ms: spawned.for_ms,
                    previous: spawned.previous,
                    started: now,
                    finished: false,
                    last_progress: now,
                    client: Some(ParkedClient {
                        stream,
                        outbound,
                        _slot: slot,
                    }),
                    revert_guard: None,
                });
                Ok(())
            }
            // The checks and the spawn touch nothing of the connection,
            // so a refusal hands its queue and slot straight back: the
            // caller puts them back before answering, and nothing queued
            // is lost, no table capacity leaks. Boxed: the queue it
            // carries makes this far too big for an inline `Err`.
            Err(response) => Err(Box::new(LookApplyRefused {
                response,
                outbound,
                slot,
            })),
        }
    }

    /// Validates a `look_apply` request and spawns its child, touching
    /// nothing of the requesting connection: the park (which takes the
    /// queue and the slot) runs only on success, in `start_look_apply`.
    fn check_and_spawn_look_apply(
        &mut self,
        name: String,
        for_ms: Option<u64>,
    ) -> Result<SpawnedLookApply, String> {
        valid_look_apply_name(&name)?;
        // Zero holds nothing: a stay, not an instant revert.
        let for_ms = for_ms.filter(|ms| *ms > 0);
        if let Some(running) = self.pending_look.as_ref() {
            return Err(format!(
                "refused: a look swap (`{}`) is already running; retry when it answers",
                running.name,
            ));
        }
        let Some(state_dir) = self.look_state_dir() else {
            return Err(
                "no look state directory: neither XDG_STATE_HOME nor HOME is set".to_owned(),
            );
        };
        // A timed preview with nothing to revert to refuses before
        // anything spawns: the apply would succeed and the preview would
        // silently never come back.
        let previous = if for_ms.is_some() {
            match scoot_look::state::read_record(&state_dir) {
                scoot_look::state::RecordRead::Present(record) => Some(record.name),
                scoot_look::state::RecordRead::Missing
                | scoot_look::state::RecordRead::Corrupt(_) => {
                    return Err(
                        "refused: `for_ms` asks for a timed preview but no look is recorded yet; \
                         swap once without it first"
                            .to_owned(),
                    );
                }
            }
        } else {
            None
        };
        let (pid, stdout) = self
            .spawn_apply_child(&name, &[])
            .map_err(|error| format!("could not start `scoot look apply {name}`: {error}"))?;
        tracing::info!(%name, pid, "started `scoot look apply` for an IPC swap");
        Ok(SpawnedLookApply {
            pid,
            stdout,
            name,
            for_ms,
            previous,
        })
    }

    /// Spawns `scoot look apply NAME --from-compositor --json` (plus
    /// `extra` args) with the session environment: the binary this
    /// compositor runs as (so a renamed or relocated install still finds
    /// its own tool), the live socket and display, and this session's
    /// look state. Stdout piped (the reply's source); stderr inherited
    /// (the child's notes land in this log); stdin null. Dropping the
    /// `Child` neither waits nor kills: the SIGCHLD reaper collects the
    /// exit.
    fn spawn_apply_child(
        &self,
        name: &str,
        extra: &[&str],
    ) -> std::io::Result<(u32, std::process::ChildStdout)> {
        let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("scoot"));
        let mut child = self.session_command(program.as_os_str());
        child
            .arg("look")
            .arg("apply")
            .arg(name)
            .arg("--from-compositor")
            .args(extra)
            .arg("--json");
        child.stdin(std::process::Stdio::null());
        child.stdout(std::process::Stdio::piped());
        let mut child = child.spawn()?;
        let pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no piped stdout from the apply child"))?;
        set_nonblocking(&stdout)?;
        drop(child);
        Ok((pid, stdout))
    }

    /// Reaps the apply child, if it exited. Runs from the SIGCHLD drain
    /// (`reap_children`), so completion lands on every backend without
    /// waiting for a frame tick.
    pub(crate) fn reap_look_apply(&mut self) {
        let pid = match self.pending_look.as_ref().and_then(|p| p.pid) {
            Some(pid) => pid,
            None => return,
        };
        let waited = super::super::child_reaper::wait(pid);
        if matches!(waited, super::super::wallpaper::Waited::Running) {
            return;
        }
        self.finish_look_apply(ChildEnd::from_waited(waited));
    }

    /// Turns the reaped child's exit into the parked reply (or a log, for
    /// a timer revert), and arms the preview timer on a timed success. A
    /// revert stashed while this swap ran goes next: the generation guard
    /// is re-read, so a swap that landed (or a newer choice) stands it
    /// down, while a swap that rolled back to the preview still reverts.
    fn finish_look_apply(&mut self, end: ChildEnd) {
        let mut pending = match self.pending_look.take() {
            Some(pending) => pending,
            None => return,
        };
        pending.pid = None;
        let output = pending
            .stdout
            .as_mut()
            .map(read_child_output)
            .unwrap_or_default();
        pending.stdout = None;
        pending.finished = true;
        pending.last_progress = Instant::now();
        let response = look_applied_from_child(&pending.name, end, &output);
        // A timed preview's revert arms only on a swap that actually
        // landed, identified by the generation its own reply carries.
        // Without one there is nothing to guard on and no timer arms; a
        // failed preview has nothing to revert (its Error says why).
        if let (Some(for_ms), Some(previous)) = (pending.for_ms, pending.previous.clone()) {
            match (&response, child_generation(&output)) {
                (Response::LookApplied { .. }, Some(generation)) => {
                    self.arm_look_revert(previous, generation, for_ms);
                }
                (Response::LookApplied { .. }, None) => tracing::warn!(
                    "a timed look preview landed without a generation; not scheduling its revert"
                ),
                _ => {}
            }
        }
        // A timer revert's child that failed on a still-held lock waits
        // out the holder like a deferred revert, bounded: `settle_look`
        // re-runs the stash below, and each round re-checks the
        // generation. Anything else (a success, a superseded stand-down,
        // a genuine refusal) is already answered in the log below.
        let revert_lockout = match (&pending.revert_guard, &response) {
            (Some(guard), Response::Error { message }) if is_lock_contention(message) => {
                Some(*guard)
            }
            _ => None,
        };
        match revert_lockout {
            Some(guard) if guard.retries > 0 => {
                let Some(dir) = self.look_state_dir() else {
                    tracing::warn!(
                        "a timed look preview's revert is locked out with no state dir; giving up loudly"
                    );
                    self.answer_look_apply(pending, response);
                    self.run_deferred_look_revert();
                    return;
                };
                self.deferred_look_revert = Some(DeferredLookRevert {
                    previous: pending.name.clone(),
                    generation: guard.generation,
                    state_dir: dir,
                    retries: guard.retries - 1,
                });
                tracing::info!(
                    "a timed look preview's revert is locked out; it waits out the holder"
                );
                self.answer_look_apply(pending, response);
                self.run_deferred_look_revert();
                return;
            }
            Some(_) => {
                if let Response::Error { message } = &response {
                    tracing::warn!(
                        %message,
                        "a timed look preview's revert is still locked out; giving up loudly"
                    );
                }
            }
            None => {}
        }
        self.answer_look_apply(pending, response);
        self.run_deferred_look_revert();
    }

    /// Queues the parked reply behind the connection's inherited tail, and
    /// drops the entry when the socket takes it whole. A part-written
    /// reply stays parked for the frame tick; a dead peer drops it there.
    fn answer_look_apply(&mut self, mut pending: PendingLookApply, response: Response) {
        let line = match encode(&response) {
            Ok(line) => line,
            // `LookApplied` is strings and `Error` one string; serde
            // cannot fail on either. Nothing to answer with if it did.
            Err(_) => return,
        };
        match pending.client.as_mut() {
            Some(client) => {
                if client.outbound.send(&mut &client.stream, line).is_err() {
                    tracing::debug!("dropped a look reply its client never read");
                    return;
                }
                if client.outbound.is_empty() {
                    tracing::info!(name = %pending.name, "answered an IPC look swap");
                    return;
                }
                self.ensure_ticking();
                self.pending_look = Some(pending);
            }
            None => {
                // A timer revert answers nobody: its outcome is a log line.
                // A superseded stand-down is expected (a newer choice
                // landed after the guard was read), not a failure.
                match response {
                    Response::LookApplied { name, .. } => {
                        tracing::info!(%name, "a timed look preview reverted");
                    }
                    Response::Error { message }
                        if message.starts_with(scootctl::look::apply::PREVIEW_SUPERSEDED) =>
                    {
                        tracing::info!(%message, "a timed look preview stood down");
                    }
                    Response::Error { message } => {
                        tracing::warn!(%message, "a timed look revert refused");
                    }
                    _ => {}
                }
            }
        }
    }

    /// Arms the timed-preview revert: after `for_ms`, swap back to
    /// `previous` unless the recorded generation moved.
    fn arm_look_revert(&mut self, previous: String, generation: u64, for_ms: u64) {
        let state_dir = match self.look_state_dir() {
            Some(dir) => dir,
            None => {
                tracing::warn!("a timed look preview landed with no state dir; not reverting it");
                return;
            }
        };
        let deferred = DeferredLookRevert {
            previous,
            generation,
            state_dir,
            retries: DEFERRED_REVERT_RETRIES,
        };
        match self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(for_ms)),
            // `FnMut`, not `FnOnce`: calloop may poll the source more
            // than once, so the owned captures clone per fire (it drops
            // itself after the first, so this runs once).
            move |_, _, state: &mut State| {
                let DeferredLookRevert {
                    previous,
                    generation,
                    state_dir,
                    retries,
                } = deferred.clone();
                state.revert_look_preview(previous, generation, state_dir, retries);
                TimeoutAction::Drop
            },
        ) {
            Ok(_) => tracing::info!(for_ms, "a timed look preview will revert"),
            Err(error) => tracing::warn!(
                %error,
                "could not arm a timed look preview's revert; the swap stays"
            ),
        }
    }

    /// Runs the stashed deferred revert, if any: the generation guard is
    /// re-read, so whatever finished (or timed out) since the timer fired
    /// decides. A newer choice stands the revert down; a swap that rolled
    /// back to the preview still reverts.
    fn run_deferred_look_revert(&mut self) {
        let Some(deferred) = self.deferred_look_revert.take() else {
            return;
        };
        self.revert_look_preview(
            deferred.previous,
            deferred.generation,
            deferred.state_dir,
            deferred.retries,
        );
    }

    /// Runs a timed preview's revert, if the choice has not moved since. A
    /// swap still running does not drop the revert: it is stashed until
    /// the swap finishes (or times out), then re-checked. Anything parked
    /// -- a running child, or a reply still draining to a slow client --
    /// holds the revert: spawning over a draining entry would drop its
    /// queued tail, so the drain completes first (see `settle_look`).
    fn revert_look_preview(
        &mut self,
        previous: String,
        generation: u64,
        state_dir: PathBuf,
        retries: u8,
    ) {
        let current = match scoot_look::state::read_record(&state_dir) {
            scoot_look::state::RecordRead::Present(record) => Some(record.generation),
            scoot_look::state::RecordRead::Missing | scoot_look::state::RecordRead::Corrupt(_) => {
                None
            }
        };
        if !should_revert(current, generation) {
            tracing::info!(
                "a timed look preview's generation moved; leaving the newer choice alone"
            );
            return;
        }
        if self.pending_look.is_some() {
            tracing::info!(
                "a look swap is still parked; a timed look preview's revert waits for it"
            );
            self.deferred_look_revert = Some(DeferredLookRevert {
                previous,
                generation,
                state_dir,
                retries,
            });
            return;
        }
        // The guard travels with the child: a choice that lands between
        // this read and the child's own swap refuses as superseded
        // inside the child's lock, instead of being undone.
        let guard = format!("--only-if-generation={generation}");
        let name = previous.clone();
        match self.spawn_apply_child(&previous, &[guard.as_str()]) {
            Ok((pid, stdout)) => {
                tracing::info!(%name, pid, "reverting a timed look preview");
                let now = Instant::now();
                self.pending_look = Some(PendingLookApply {
                    pid: Some(pid),
                    stdout: Some(stdout),
                    name,
                    for_ms: None,
                    previous: None,
                    started: now,
                    finished: false,
                    last_progress: now,
                    client: None,
                    revert_guard: Some(RevertGuard {
                        generation,
                        retries,
                    }),
                });
                self.ensure_ticking();
            }
            Err(error) => {
                tracing::warn!(%error, "could not start a timed look preview's revert");
            }
        }
    }

    /// Gives up on an apply child past its bound: answers the parked
    /// request with an error, and hands the pid to the tracked-children
    /// sweep so its exit is still reaped (and forgotten) later. The child
    /// keeps running -- see [`LOOK_APPLY_TIMEOUT`].
    fn timeout_look_apply(&mut self) {
        let mut pending = match self.pending_look.take() {
            Some(pending) => pending,
            None => return,
        };
        if let Some(pid) = pending.pid.take() {
            self.spawned_children.insert(pid);
        }
        pending.stdout = None;
        pending.finished = true;
        pending.last_progress = Instant::now();
        tracing::warn!(
            name = %pending.name,
            timeout_s = LOOK_APPLY_TIMEOUT.as_secs(),
            "a look swap child has not finished; no longer waiting on it (it keeps running, \
             and its exit is reaped when it comes)"
        );
        self.answer_look_apply(
            pending,
            refused(
                "timed out waiting for the look swap to finish; \
                 `scoot look current` says which look is active"
                    .to_owned(),
            ),
        );
        // A revert stashed while this swap ran stays stashed: the child
        // keeps running past its bound, and `settle_look` runs the stash
        // below once nothing is parked, re-checking the generation then.
    }

    /// Moves the parked swap along by one frame tick: times out a hung
    /// child (an exited one is reaped from the SIGCHLD drain instead, so
    /// completion never waits for a tick), and drains a queued reply.
    /// Returns whether work remains -- a frame-timer re-arm condition.
    pub fn settle_look(&mut self) -> bool {
        if let Some(pending) = self.pending_look.as_ref()
            && !pending.finished
            && Instant::now().duration_since(pending.started) >= LOOK_APPLY_TIMEOUT
        {
            self.timeout_look_apply();
        }
        // Drain a queued reply. Decided under the borrow, cleared after
        // it: the drain borrows the entry it may finish.
        let mut clear = false;
        if let Some(pending) = self.pending_look.as_mut()
            && pending.finished
        {
            match pending.client.as_mut() {
                // A finished revert holds no reply to drain.
                None => clear = true,
                Some(client) => {
                    let before = client.outbound.pending();
                    match client.outbound.flush(&mut &client.stream) {
                        Ok(()) => {
                            if client.outbound.pending() < before {
                                pending.last_progress = Instant::now();
                            }
                            if client.outbound.is_empty() {
                                clear = true;
                            } else if Instant::now().duration_since(pending.last_progress)
                                >= LOOK_REPLY_STALL
                            {
                                tracing::warn!(
                                    "gave up writing a look reply: the client stopped reading"
                                );
                                clear = true;
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, "dropped a look reply its client never read");
                            clear = true;
                        }
                    }
                }
            }
        }
        if clear {
            self.pending_look = None;
        }
        // A reply that just finished draining may release a stashed
        // revert: spawning over the draining entry would have dropped its
        // queued tail, so the stash waited for exactly this.
        if self.pending_look.is_none() {
            self.run_deferred_look_revert();
        }
        self.pending_look.is_some()
    }
}

/// Makes a file non-blocking, so reading the child's pipe never parks the
/// loop thread on a grandchild that inherited it.
fn set_nonblocking(file: &std::process::ChildStdout) -> std::io::Result<()> {
    use std::os::fd::AsFd as _;
    let flags = rustix::fs::fcntl_getfl(file.as_fd()).map_err(std::io::Error::from)?;
    rustix::fs::fcntl_setfl(file.as_fd(), flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(std::io::Error::from)?;
    Ok(())
}
