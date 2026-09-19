//! The control socket: newline-delimited JSON, one connection per client.
//!
//! Every byte in and out of here moves on the compositor's *only* thread, the
//! one that also runs wayland dispatch, input and the render loop -- so
//! nothing in this module may block, ever. A blocking read of half a request
//! line, or a blocking write to a client that has stopped reading, does not
//! stall one connection: it stalls the whole compositor, for every client, for
//! as long as the offender likes. Both used to happen (see
//! `docs/roadmap/10-ipc-connection-loop.md`). The shape that replaces them:
//!
//! - the accepted socket is non-blocking, and a line that has only half
//!   arrived leaves its bytes in [`line::Lines`] to be finished on a later
//!   wakeup ([`line::LineRead::Incomplete`]);
//! - one wakeup answers *every* request already buffered, not just the first,
//!   because a level-triggered readiness source will not fire again for bytes
//!   that have already left the kernel -- and stops there, so whatever is still
//!   in the kernel waits its turn behind every other source;
//! - a reply the socket will not take in one go waits in [`outbound::Outbound`]
//!   and goes out when the event loop reports the socket writable, with the
//!   connection's read interest dropped while too much is queued so it cannot
//!   be made to buffer without bound.
//!
//! This file holds the socket's setup and what a request *means*
//! ([`State::handle_request`]); [`connection`] holds the event-loop machinery
//! that gets requests in and replies out.

pub(super) mod accept;
mod connection;
mod line;
mod listener;
mod outbound;
mod slots;
#[cfg(test)]
mod tests;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use scoot_core::{Action, WindowId};
use scoot_ipc::{
    OutputSnapshot, PROTOCOL_VERSION, Rect as WireRect, Request, Response, WindowSnapshot, encode,
    socket_path,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode};

use self::connection::Limits;
pub(crate) use self::outbound::Outbound;
use self::slots::{MAX_CONNECTIONS, Slot, Slots};
use super::State;
use super::fd_pressure::{RESERVE_FDS, Table};
use super::headless::FRAME_INTERVAL;
use super::tty::VtSwitchOutcome;

/// A `wait-idle` request that hasn't been answered yet.
///
/// Its connection has already left the event loop (see the `WaitIdle` arm of
/// `Connection::serve`), so this carries everything needed to finish with it
/// from the render loop: the socket, when the request started and how long it
/// may run, and the outbound queue it inherited.
pub struct PendingIdle {
    stream: UnixStream,
    /// How long the screen must stay unchanged before this is answered.
    quiet: Duration,
    /// How long the client is prepared to wait in total -- both for the screen
    /// to settle and, afterwards, for its answer to be written (see
    /// [`PendingIdle::push`]).
    timeout: Duration,
    started: Instant,
    /// When a byte of this waiter's queue last went out -- or, if none has,
    /// when its answer was queued. `started` until one of those happens.
    last_progress: Instant,
    /// Whether this request's own answer has been decided and queued.
    ///
    /// Not derivable from `outbound`: that is also non-empty *before* the
    /// answer exists, when it holds a tail inherited from the connection, and
    /// empty again once the answer has gone out.
    answered: bool,
    /// This request's answer on its way out, plus -- ahead of it -- whatever
    /// the connection had not finished writing when it handed over.
    outbound: Outbound,
    /// The connection slot this waiter inherited, released when it is done
    /// with (answered and written, or given up on). Held, never read: a
    /// waiter's fd is one of the [`MAX_CONNECTIONS`] this compositor serves
    /// just as much as a live connection's is. `Option` only because that is
    /// how it moves out of the connection handing over; always `Some` here.
    _slot: Option<Slot>,
}

/// Identifies an IPC connection to `State` for as long as it lives.
///
/// What a screenshot parked by a connection is filed under (see
/// `screenshot.rs`), so its reply finds its way back after `serve` has
/// returned. Never reused within a process -- a wrapping counter could hand
/// a late completion to a connection that happened to inherit its number, so
/// this starts at 1 and a `u64` simply never wraps in practice. Relaxed
/// ordering: assigned once per accept, never read-modify-written against
/// anything else.
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// The most characters one `Request::Type` may carry.
///
/// A separate, smaller cap than [`line::MAX_REQUEST_BYTES`]: each character
/// becomes two to four key events run synchronously on the event-loop thread,
/// so a megabyte of text stalls wayland dispatch, input and rendering for
/// seconds (see `docs/backlog/resolved/msg-type-blocks-event-loop-resolved.md`). Sized by
/// measurement, not feel: release `--headless` on the dev VM types 50,000
/// characters in ~99ms all-lowercase and ~215ms all-shifted, i.e. ~2us and
/// ~4.3us per character, so 16,384 worst-case characters cost ~75ms --
/// comfortably sub-second -- while a real shell command line (a few hundred
/// characters, under a millisecond) never notices it.
///
/// Counted in characters, not bytes: a character is the cost unit (each one
/// becomes key events), whatever its UTF-8 length. Even 16,384 four-byte
/// characters encode to well under the 1 MiB line limit, so this cap always
/// fires first -- the layering is deliberate, not incidental.
pub(super) const MAX_TYPE_CHARS: usize = 16 * 1024;

/// What a client past [`MAX_CONNECTIONS`] is told, encoded once.
///
/// A constant answer to a constant question, so it is built on the first
/// refusal and reused: the path is not hot (it takes a client in a connect
/// loop to reach it at all), but a fresh `format!` and `encode` per refusal
/// would be allocation handed out in response to exactly the behaviour this
/// bound exists to discourage.
static REFUSAL: LazyLock<String> = LazyLock::new(|| {
    encode(&Response::error(format!(
        "refused: scoot serves at most {MAX_CONNECTIONS} ipc connections at once, \
         and every slot is in use. Close one, or send this request on a connection \
         that is already open -- requests pipeline, so one connection is enough for \
         any number of them"
    )))
    // `Response::Error` is one `String`; serde cannot fail on it. An empty
    // line would simply mean a refused client is closed without a reason.
    .unwrap_or_default()
});

/// What a client arriving under compositor-wide fd pressure is told,
/// encoded once.
///
/// The same refused-with-a-reason shape as [`REFUSAL`] above, for the same
/// reason: unlike a Wayland connection, this channel can carry why. The
/// advice differs (retry shortly, not close-and-reuse: pressure lifts when
/// whoever is holding fds lets go, and holding connections is not what
/// caused it), and so does the log level at the refusal site (`warn`,
/// matching the accept-loop shed: pressure means the table is nearly full,
/// not merely that a cap is).
static PRESSURE_REFUSAL: LazyLock<String> = LazyLock::new(|| {
    encode(&Response::error(format!(
        "refused: scoot is under file-descriptor pressure (fewer than {RESERVE_FDS} \
         fds free); retry in a moment -- this connection cost nothing, and pressure \
         lifts as soon as whoever is holding fds lets go"
    )))
    // Same infallibility argument as [`REFUSAL`].
    .unwrap_or_default()
});

pub fn init(
    event_loop: &mut EventLoop<'static, State>,
    state: &mut State,
    socket: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = socket
        .or_else(socket_path)
        .ok_or("no socket path: set SCOOT_SOCKET or XDG_RUNTIME_DIR")?;
    let listener = listener::bind(&path)?;

    // Owned by the accept loop rather than by `State`: the count is nobody
    // else's business, and every live connection holds its own claim on it
    // (see `slots`).
    let slots = Slots::new();
    // The spare fd the accept loop spends to shed a pending connection when
    // the process is out of fds (see `accept`): one fd held for the session,
    // owned here for the same reason as the slots.
    let spare = accept::Spare::new();
    event_loop.handle().insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, state: &mut State| {
            Ok(accept::drain(listener, &spare, &mut |stream| {
                if let Err(error) = accept(state, stream, &slots, Limits::REAL) {
                    tracing::warn!(%error, "could not take an ipc client");
                }
            }))
        },
    )?;

    state.ipc_path = Some(path);
    Ok(())
}

/// Takes one accepted socket into the event loop, or refuses it.
///
/// `limits` is [`Limits::REAL`] everywhere but the tests, which pass shorter
/// deadlines rather than parking a test thread for tens of seconds. Reads
/// the live fd table and delegates; the tests drive [`accept_under`]
/// directly with canned readings.
fn accept(
    state: &mut State,
    stream: UnixStream,
    slots: &Slots,
    limits: Limits,
) -> std::io::Result<()> {
    accept_under(state, stream, slots, limits, super::fd_pressure::table())
}

/// The testable half of [`accept`]: the same checks against a given table
/// reading instead of the live one.
///
/// Order matters: the peer check first (security, before this connection
/// costs anything), then non-blocking (every refusal below writes, and
/// that write must not block the compositor either), then the pressure
/// refusal (a pressured table must not spend a slot it cannot serve --
/// and, unlike the cap refusal past it, this one names the pressure
/// rather than the newcomer's own behaviour), then the slot cap. An
/// unknown table (`None`) skips the pressure refusal: shedding on a
/// broken gauge would deny innocents, and the `EMFILE` shed still catches
/// real exhaustion underneath.
fn accept_under(
    state: &mut State,
    stream: UnixStream,
    slots: &Slots,
    limits: Limits,
    table: Option<Table>,
) -> std::io::Result<()> {
    // First, before this connection costs anything: an fd duplicated, a
    // buffer allocated, a place in the event loop -- and long before any
    // request of its own is read. A client that is not this compositor's own
    // user gets nothing but a closed socket.
    let peer = listener::peer_uid(&stream)?;
    let own = listener::own_uid();
    if !listener::peer_is_allowed(peer, own) {
        // Loud on purpose. This channel injects keystrokes and hands back
        // screenshots, so somebody else's process reaching it at all is
        // worth a trace at the default log level, the same way a discarded
        // VT switch is (see `tty::change_vt`) -- it is refused, not an
        // error, but it is never routine.
        tracing::warn!(
            uid = peer,
            expected_uid = own,
            "refused an ipc connection from another user",
        );
        return Ok(());
    }

    // Non-blocking is what every read and write in `connection` is written
    // for, and it is not inherited: on Linux `accept(2)` gives a fresh socket
    // its own file status flags, and std asks for `SOCK_CLOEXEC` only.
    // Verified on the dev VM rather than assumed -- with the listener itself
    // already non-blocking, `fcntl(F_GETFL) & O_NONBLOCK` on the accepted fd
    // reads false. Set explicitly either way: this is load-bearing enough that
    // it should not depend on what the listener happens to be. Before the
    // refusal below as well as after it, so that refusal's one write cannot
    // block the compositor either.
    stream.set_nonblocking(true)?;

    if let Some(observed) = table.filter(|observed| observed.pressured()) {
        // Refused outright like an over-cap connection, and told why like
        // one: retry shortly, not close-and-reuse -- pressure is nobody's
        // behaviour, it lifts on its own. Best-effort write for the same
        // reason as the cap refusal below. No slot is claimed, so the
        // refusal costs the table nothing.
        tracing::warn!(
            used = observed.used,
            soft = observed.soft,
            free = observed.free(),
            "refused an ipc connection: file-descriptor pressure"
        );
        let _ = (&stream).write_all(PRESSURE_REFUSAL.as_bytes());
        return Ok(());
    }

    let Some(slot) = slots.claim() else {
        // Refused outright rather than queued: a client waiting for a slot
        // would be an unbounded list of waiting clients instead of an
        // unbounded list of connections, and a retry costs a millisecond.
        // Told why, rather than handed a socket that closes for no stated
        // reason -- an agent that hits this needs to know it is the one
        // holding them all. Best-effort, because there is nothing to do about
        // a refusal that cannot be written, and reliable in practice: a
        // freshly accepted socket's buffer is empty and this line is short.
        tracing::debug!(
            max = MAX_CONNECTIONS,
            "refused an ipc connection: every connection slot is taken"
        );
        let _ = (&stream).write_all(REFUSAL.as_bytes());
        return Ok(());
    };

    state
        .loop_handle
        .insert_source(
            connection::source(
                stream,
                slot,
                limits,
                NEXT_CONN.fetch_add(1, Ordering::Relaxed),
            )?,
            |_readiness, connection, state: &mut State| connection.step(state),
        )
        // The message rather than the error itself: an `InsertError` carries
        // the source back out, and a source is single-threaded (it holds the
        // connection's slot and its stall deadline, both `Rc`-backed), while
        // `io::Error::other` wants something `Send + Sync`. Dropping it here
        // is also what releases the slot the connection never got to use.
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(())
}

impl State {
    pub fn handle_request(&mut self, request: Request) -> Response {
        match request {
            Request::Version => Response::Version {
                version: env!("CARGO_PKG_VERSION").to_string(),
                protocol: PROTOCOL_VERSION,
            },
            Request::Outputs => Response::Outputs {
                outputs: self.output_snapshots(),
            },
            Request::Windows => Response::Windows {
                windows: self.window_snapshots(),
            },
            // Refused while the session is locked, and it is the only
            // request here that is. `Action` is the one that bypasses input
            // entirely -- `Spawn` would put a new client's window on a locked
            // screen, `Quit` would tear the session down, and every layout
            // action would move windows the user cannot see. The injected
            // *input* requests below are deliberately still served: they go
            // through the same focus and hit-test paths a real keyboard and
            // mouse do, so while locked they can only reach the lock surface
            // (see `session_lock.rs`), which is what lets an agent drive a
            // lock screen exactly as a person would.
            Request::Action(action) => {
                if self.session_lock.is_locked() {
                    return Response::error(
                        "refused: the session is locked; window-management actions are not \
                         available until the lock client unlocks it (injected keyboard and \
                         pointer input still works, and reaches only the lock screen)",
                    );
                }
                // A click that gave an `on_demand` layer surface the keyboard
                // is spent here when the action moves window focus -- exactly
                // what `input.rs`'s `focus_under_pointer` does on the line
                // before its own `act(FocusWindowId)`, and what
                // `wlr_toplevel_activate` and `request_activation` do for
                // their own focus requests. Without it `layer_shell.rs`'s
                // `layer_keyboard_focus` hands the keyboard straight back to
                // that still-mapped surface, so the refresh `act` ends in
                // re-derives the panel instead of the window: `scoot msg
                // windows` reports the new focus while every keystroke still
                // goes to the panel, with nothing to detect the mismatch
                // from -- and unlike an `xdg-activation-v1` launcher, nothing
                // here unmaps itself a moment later to self-correct. Only the
                // actions whose purpose is moving window focus spend it: a
                // layout or lifecycle action (`MoveColumn`, `CloseFocused`,
                // ...) changes arrangement rather than where focus is
                // reported to be, so it leaves a deliberate keyboard placement
                // alone. After the lock gate above, for the same reason as
                // there: a refused request must not spend the click, so the
                // session comes back as the user left it.
                if matches!(
                    &action,
                    scoot_ipc::Action::FocusColumn { .. }
                        | scoot_ipc::Action::FocusWindow { .. }
                        | scoot_ipc::Action::FocusWindowId { .. }
                        | scoot_ipc::Action::FocusWorkspace { .. }
                        | scoot_ipc::Action::FocusWorkspaceIndex { .. }
                ) {
                    self.clicked_layer = None;
                }
                // The already-there fast path `ext_workspace.rs`'s
                // `commit_workspace_requests` and `wlr_toplevel_activate`
                // already have: a client repeating a focus action for the
                // target it is already on would otherwise drive a full
                // `apply` -- an arrange, a configure per window, a render --
                // as fast as it can write to its socket. Skipped is only
                // `act`; the click above is spent and the keyboard half
                // below runs on both halves, so the two agree about the
                // same gesture. The reply is `ok()` either way.
                if self.focus_action_is_noop(&action) {
                    self.refresh_keyboard_focus();
                    return self.ok();
                }
                self.act(Action::from(action));
                self.ok()
            }
            // Connection::step() intercepts and answers this variant itself
            // (see above) before handle_request is ever called: a capture is
            // dispatched to the encode worker in `serve`, and its reply comes
            // back through the completion channel after `serve` has returned.
            // If this ever fires, that interception was bypassed -- answer an
            // error rather than aborting over it (an abort here would take
            // every client's unsaved state with it over a routing bug), and
            // fail loudly in debug builds so a test catches the bypass.
            Request::Screenshot { .. } => {
                debug_assert!(
                    false,
                    "Screenshot reached handle_request; Connection::serve must intercept it"
                );
                Response::error("could not take a screenshot: internal routing error")
            }
            Request::PointerMove { x, y } => {
                self.pointer_move(x, y);
                self.ok()
            }
            Request::PointerButton { button, pressed } => {
                self.pointer_button(button, pressed);
                self.ok()
            }
            Request::Click { x, y, button } => {
                self.pointer_move(x, y);
                self.pointer_button(button, true);
                self.pointer_button(button, false);
                self.ok()
            }
            Request::Scroll { dx, dy } => {
                self.scroll(dx, dy);
                self.ok()
            }
            // Three of `press()`'s `Ok(Some(VtSwitchOutcome))` cases need
            // something other than a bare `Ok` -- each for a different
            // reason, so each gets its own arm rather than folding them
            // together:
            //
            // - `Requested`: this exact call just issued a real VT_ACTIVATE
            //   for a VT other than the one displayed (the same-VT no-op
            //   below is filtered out before libseat is called, so reaching
            //   libseat already means a change was asked for) that libseat
            //   didn't reject outright, so this IPC connection -- if it's
            //   the caller's only input path -- may be about to lose the one
            //   channel that could switch back (see
            //   `tty::VtSwitchOutcome`'s doc and the backlog item this
            //   closes in `docs/roadmap/05b-vt-switch-eperm.md`). Worded as "requested"/"if it takes
            //   effect," not "this switched" -- libseat's own docs say a
            //   successful switch_session call doesn't guarantee a switch
            //   happens, so claiming a definite pause here would overclaim.
            // - `IgnoredSameVt`: the request named the VT already displayed
            //   -- verified against the kernel before libseat was called, so
            //   there is nothing to hedge about and nothing to warn about.
            //   A plain `Ok`: warning here would be the false positive
            //   `docs/backlog/resolved/vt-switch-same-vt-warning-done.md`
            //   exists to suppress.
            // - `IgnoredPaused`: the request went nowhere (libseat was never
            //   asked), but *why* matters to an IPC caller specifically --
            //   this is the one-way-door scenario itself: an agent retrying
            //   its switch-back combo over IPC while paused needs to hear
            //   "this can't work from here," not a bare `Ok` indistinguishable
            //   from a real switch-back actually working (see 5b/5c in
            //   `docs/roadmap/05b-vt-switch-eperm.md` for why that ambiguity is
            //   exactly the defect).
            // - `Failed`: libseat itself returned an error -- a request that
            //   did not succeed, so unlike `Requested` this isn't "success
            //   with a side effect," it's a plain failure and gets
            //   `Response::error` rather than `Warning`.
            //
            // Plain `Ignored` (no `--tty` backend at all -- this request
            // means nothing on this backend, nothing to warn about) and
            // `None` (no VT binding matched at all) stay a plain `Ok`.
            Request::Key { keys } => match self.press(&keys) {
                Ok(Some(VtSwitchOutcome::Requested)) => Response::Warning {
                    message: "requested a VT switch away from --tty (libseat \
                              does not guarantee this actually happens, e.g. \
                              it is a no-op if already on the target VT); if \
                              it takes effect, the compositor will be paused, \
                              and only a VT switch from outside this \
                              compositor -- a physical Ctrl+Alt+Fn, or \
                              `sudo chvt N` from a shell on this machine -- \
                              not IPC, can reactivate it"
                        .to_string(),
                },
                Ok(Some(VtSwitchOutcome::IgnoredPaused)) => Response::Warning {
                    message: "ignored: this session is already paused; a VT \
                              switch from outside this compositor -- a \
                              physical Ctrl+Alt+Fn, or `sudo chvt N` from a \
                              shell on this machine -- is needed before a VT \
                              switch can succeed from here; other IPC \
                              requests still work"
                        .to_string(),
                },
                Ok(Some(VtSwitchOutcome::Failed)) => Response::error(
                    "requested a VT switch away from --tty, but libseat \
                     rejected the request (see the compositor log for why)",
                ),
                // Spelled out rather than `Ok(_)`: a collapsed catch-all is
                // exactly the shape of bug that made the paused-retry case
                // silently indistinguishable from success above (see
                // `VtSwitchOutcome::Ignored`'s doc) -- keeping this
                // exhaustive means a future sixth `VtSwitchOutcome` variant
                // fails to compile here instead of silently becoming `Ok`.
                Ok(None)
                | Ok(Some(VtSwitchOutcome::Ignored))
                | Ok(Some(VtSwitchOutcome::IgnoredSameVt)) => self.ok(),
                Err(error) => Response::error(error),
            },
            Request::Type { text } => {
                // Refused rather than delayed, the same shape the screenshot
                // rate limit took, and for the same reason: this runs to
                // completion synchronously on the event-loop thread, so
                // waiting (or chunking, which needs the per-connection
                // progress state item 10 deliberately avoided) is not on
                // offer. Checked before `type_text`, so a refused request
                // types nothing -- not even a prefix.
                let chars = text.chars().count();
                if chars > MAX_TYPE_CHARS {
                    return Response::error(format!(
                        "refused: `type` carries at most {MAX_TYPE_CHARS} characters per \
                         request (this one has {chars}), because each character becomes \
                         key events typed synchronously on the event-loop thread. Split \
                         the text across several `type` requests"
                    ));
                }
                match self.type_text(&text) {
                    Ok(()) => self.ok(),
                    Err(error) => Response::error(error),
                }
            }
            // Connection::step() intercepts and answers this variant itself
            // (see above) before handle_request is ever called, since a
            // reply here has to wait on pending_idle instead of being
            // returned immediately like every other request. If this ever
            // fires, that interception was bypassed -- a bug worth a loud
            // failure, not a request that quietly appears to work while
            // answering wrong.
            Request::WaitIdle { .. } => unreachable!("WaitIdle is answered by Connection::step"),
        }
    }

    /// Answers `wait-idle` requests whose quiet period has passed, or timed
    /// out, and pushes out the ones whose answer didn't fit in one write.
    pub fn settle_idle_waiters(&mut self) {
        if self.pending_idle.is_empty() {
            return;
        }
        let now = Instant::now();
        let last_commit = self.last_commit;
        let mut waiting = std::mem::take(&mut self.pending_idle);
        waiting.retain_mut(|wait| wait.advance(now, last_commit));
        self.pending_idle = waiting;
    }

    pub(super) fn window_snapshots(&self) -> Vec<WindowSnapshot> {
        let arrangement = self.world.arrange();
        // Resolved once per request, not per window: which window's popup
        // tree holds the keyboard, if a live grab names one. A `windows`
        // request is an agent asking for a list, not a per-frame path, and
        // the lookup itself allocates nothing (see `popup.rs`).
        let grab_holder = self.popup_grab_holder();
        arrangement
            .placements
            .iter()
            .map(|placement| {
                let info = self
                    .world
                    .window_info(placement.id)
                    .cloned()
                    .unwrap_or_default();
                WindowSnapshot {
                    id: placement.id.0,
                    app_id: info.app_id,
                    title: info.title,
                    // Read off the surface rather than the core: the core is
                    // platform-independent and knows nothing about Wayland
                    // protocols (see `toplevel_icon.rs`). One `with_states`
                    // per window per `windows` request, which is an agent
                    // asking for a list -- not a per-frame path.
                    icon: self.icon_name_of(placement.id),
                    output: placement.output.0,
                    rect: wire(placement.rect),
                    visible: placement.visible,
                    focused: arrangement.focused == Some(placement.id),
                    popup_grab: grab_holder == Some(placement.id),
                }
            })
            .collect()
    }

    /// Whether an IPC focus action would move nothing, so `handle_request`
    /// can skip `act`'s full `apply` and run only the keyboard half.
    ///
    /// What "already there" means is defined per variant, each read off the
    /// state the core's own action would resolve against -- a wrong answer
    /// here silently drops a focus change, which is worse than a redundant
    /// arrange, so anything that cannot be answered exactly stays on the
    /// full path:
    /// - `FocusWindowId`: `State::focus` already names that window, the
    ///   same compare `wlr_toplevel_activate`'s fast path makes. An unknown
    ///   id never equals it (`remove_window` clears `focus`, and ids are
    ///   never reused), so it stays on the full path and keeps whatever
    ///   handling `act` gives it today.
    /// - `FocusWorkspaceIndex`: the focused output's active workspace
    ///   already is that index. Out of range can never equal `active`
    ///   (`active < count` always), so it stays on the full path too. Read
    ///   off the focused output rather than the primary one
    ///   `ext_workspace.rs` reads because that is the list the core's own
    ///   `reshape` resolves the index against.
    /// - `FocusWorkspace`: the step clamps -- up from the first workspace,
    ///   or down from the last. The same lookup as the index case; the core
    ///   then only re-sets the index it already has and re-normalises an
    ///   already-normalised tree, which is the reasoning PR #54's fast path
    ///   states for skipping `act` there.
    /// - `FocusColumn` / `FocusWindow`: relative steps whose no-op-ness
    ///   needs the focused column's position in its workspace (and the
    ///   stack position within it), which `World` does not expose.
    ///   Deliberately left on the full path rather than guessed; resolving
    ///   them would mean new core accessors for a socket-speed micro-opt,
    ///   and they keep today's behavior exactly, clear included.
    ///
    /// No allocation: an enum match plus, for the workspace variants, two
    /// `Copy` reads off the core.
    fn focus_action_is_noop(&self, action: &scoot_ipc::Action) -> bool {
        match action {
            scoot_ipc::Action::FocusWindowId { id } => self.focus == Some(WindowId(*id)),
            scoot_ipc::Action::FocusWorkspaceIndex { index } => self
                .world
                .focused_output()
                .and_then(|output| self.world.workspaces(output))
                .is_some_and(|workspaces| workspaces.active == *index),
            scoot_ipc::Action::FocusWorkspace { direction } => self
                .world
                .focused_output()
                .and_then(|output| self.world.workspaces(output))
                .is_some_and(|workspaces| {
                    (*direction == scoot_ipc::Vertical::Up && workspaces.active == 0)
                        || (*direction == scoot_ipc::Vertical::Down
                            && workspaces.active + 1 >= workspaces.count)
                }),
            _ => false,
        }
    }

    /// Success, carrying the session-lock state the response was built
    /// under -- see `Response::Ok`'s doc for what `locked` does and does
    /// not promise. One constructor rather than seven literals, so a new
    /// `Ok` site cannot forget the flag.
    fn ok(&self) -> Response {
        Response::Ok {
            locked: self.session_lock.is_locked(),
        }
    }

    pub(super) fn output_snapshots(&self) -> Vec<OutputSnapshot> {
        self.world
            .outputs()
            .into_iter()
            .map(|(id, area)| OutputSnapshot {
                id: id.0,
                // Each output's own `wl_output.name`, looked up by the id the
                // core reports it under -- the same string a bar or
                // `wlr-randr` would show. Empty for an id the core has but
                // this side does not, which nothing can produce today (every
                // `OutputAdded` is sent from `headless.rs` with the id
                // `Outputs::add` just returned).
                name: self
                    .outputs
                    .get(id)
                    .map(|output| output.name())
                    .unwrap_or_default(),
                rect: wire(area),
                scale: self.output_scale,
                // Unknown output (not in the core's list) reports no
                // usable area rather than a wrong one: an all-zero
                // `usable` is the documented "predates the field"
                // sentinel, and a missing output is the closest thing to
                // that this server can say.
                usable: wire(self.world.usable_area(id).unwrap_or_default()),
            })
            .collect()
    }
}

fn wire(rect: scoot_core::Rect) -> WireRect {
    WireRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    }
}

/// Whether a screenshot request arriving at `now` should be refused because
/// this connection was already handed one less than a frame ago.
///
/// `last` is when the previous capture *finished* (see the call site), so the
/// window this enforces is a gap *between* captures: one connection can cost
/// the event loop a capture no more often than the compositor already spends
/// a frame, and everything else gets that gap to be served in. A refused
/// caller waits at most one [`FRAME_INTERVAL`] and asks again.
///
/// Deliberately not justified as "the pixels cannot have changed yet" --
/// that would be a stronger claim than the code makes good on. `render()`
/// runs on demand (`needs_render`), not on frame boundaries, and this window
/// starts whenever the last capture happened to finish, so two captures a
/// frame apart can legitimately differ. Bounding the *cost* is the point.
///
/// Deliberately not a sleep: this runs on the event-loop thread, so waiting
/// here would block every other client in order to slow one down. A refusal
/// is answered in microseconds, so a client that ignores it and hammers
/// anyway costs the compositor a JSON reply per attempt, not a render.
///
/// Per connection, not global, so what it bounds on its own is one connection
/// issuing back-to-back captures; a client could otherwise reconnect for every
/// capture and get a fresh allowance each time. That is what [`MAX_CONNECTIONS`]
/// closes: reconnecting is still free, but only 64 connections can exist at
/// once, so the captures a client can extract per frame are bounded too.
///
/// `duration_since` saturates to zero rather than panicking when `last` is
/// somehow later than `now`, so a clock that fails to be monotonic makes this
/// throttle (harmlessly) rather than abort the compositor.
fn screenshot_throttled(last: Option<Instant>, now: Instant) -> bool {
    last.is_some_and(|last| now.duration_since(last) < FRAME_INTERVAL)
}

impl PendingIdle {
    /// Moves this waiter along by one frame tick: `true` to keep waiting,
    /// `false` once it is finished with (answered and written, or given up on).
    fn advance(&mut self, now: Instant, last_commit: Instant) -> bool {
        // Anything queued goes first, for the same reason the connection did
        // it first: this waiter may have inherited the tail of an earlier
        // reply, and the answer below must not overtake it.
        if !self.push(now) {
            return false;
        }
        if !self.answered {
            let response = match idle_outcome(now, last_commit, self) {
                IdleOutcome::StillWaiting => return true,
                IdleOutcome::Idle { waited_ms } => Response::Idle { waited_ms },
                IdleOutcome::TimedOut => {
                    Response::error("timed out waiting for the screen to settle")
                }
            };
            let Ok(line) = encode(&response) else {
                // `Response::Idle` and `Response::Error` are a `u64` and a
                // `String`; serde cannot fail on either. Nothing to answer
                // with if it somehow did.
                return false;
            };
            self.answered = true;
            // The no-progress window starts here, not when the request arrived.
            // Without this the window for a *timed out* waiter would be zero by
            // construction -- `last_progress` would still be `started`, and
            // `idle_outcome` only reports `TimedOut` once `timeout` has already
            // elapsed since then -- so the first tick that could not write it
            // would also be the one that gave up on it.
            self.last_progress = now;
            let PendingIdle {
                stream, outbound, ..
            } = self;
            if outbound.send(&mut &*stream, line).is_err() {
                return false;
            }
        }
        // Kept only while there is still something to write.
        !self.outbound.is_empty()
    }

    /// Pushes out as much of the queue as the socket will take. `false` when
    /// this waiter is finished with: the peer is gone, or it has stopped making
    /// room for long enough to count as gone.
    ///
    /// The socket is non-blocking (it is a `try_clone` of the connection's,
    /// which shares its file status flags -- verified, not assumed), so a
    /// write here can come up short even for an answer this small: a client
    /// that pipelined requests and never read the replies has its own receive
    /// buffer full. Retried on the next frame tick instead of dropped, because
    /// dropping it would leave the client waiting on an answer that was
    /// decided and then thrown away -- and, when there is an inherited tail
    /// ahead of it, would truncate a response mid-way.
    ///
    /// Giving up is bounded by lack of *progress*, not by total time: a client
    /// draining a multi-megabyte screenshot reply slowly is making progress and
    /// is never given up on, however long it takes. The window is as long as the
    /// `timeout_ms` the client itself asked for, measured from the last byte
    /// that went out -- or, if none ever has, from when the answer was queued
    /// (see [`PendingIdle::advance`], which is where that clock starts, and
    /// why). The client said how long it was prepared to wait on this request,
    /// and a peer that has not taken a single byte in that long is not reading.
    fn push(&mut self, now: Instant) -> bool {
        if self.outbound.is_empty() {
            return true;
        }
        let before = self.outbound.pending();
        let PendingIdle {
            stream, outbound, ..
        } = self;
        if outbound.flush(&mut &*stream).is_err() {
            return false;
        }
        if self.outbound.pending() < before {
            self.last_progress = now;
            return true;
        }
        if now.duration_since(self.last_progress) >= self.timeout {
            tracing::warn!(
                pending = self.outbound.pending(),
                "gave up writing a wait-idle reply: the client stopped reading"
            );
            return false;
        }
        true
    }
}

enum IdleOutcome {
    StillWaiting,
    Idle { waited_ms: u64 },
    TimedOut,
}

/// Whether a `wait-idle` request should be answered yet.
///
/// "Idle" means `quiet` has elapsed with no commit *since the request was
/// made* -- not merely since whenever the last commit happened to be. Using
/// `last_commit` alone would let a request reply idle on the very first tick
/// whenever the client was already commit-idle when it arrived, before the
/// client has had any chance to react to input sent moments earlier: the
/// same "stale screenshot" race the flush bugs produced, but from a missing
/// baseline instead of a missing flush. So the quiet window is measured from
/// `last_commit.max(wait.started)`, which only equals `last_commit` once a
/// commit has actually happened after the request began.
///
/// The timeout is kept as a duration since `started` rather than a precomputed
/// deadline `Instant`, which is overflow-free by construction: `Instant +
/// Duration` panics if it overflows, and `timeout_ms` is a client-chosen `u64`
/// that goes straight into a `Duration`. Nothing was actually reachable there
/// (Linux's `Instant` is a `timespec` whose `tv_sec` is an `i64`, which
/// `u64::MAX` milliseconds fits inside with room to spare), so this is not a
/// fixed bug -- it is one fewer client-controlled value feeding arithmetic that
/// can panic at all.
fn idle_outcome(now: Instant, last_commit: Instant, wait: &PendingIdle) -> IdleOutcome {
    let baseline = last_commit.max(wait.started);
    if now.duration_since(baseline) >= wait.quiet {
        let waited_ms = now.duration_since(wait.started).as_millis() as u64;
        IdleOutcome::Idle { waited_ms }
    } else if now.duration_since(wait.started) >= wait.timeout {
        IdleOutcome::TimedOut
    } else {
        IdleOutcome::StillWaiting
    }
}
