//! The accept loop: draining the listening socket without spinning.
//!
//! The listener is registered [`Mode::Level`](smithay::reexports::calloop::Mode),
//! so a connection still sitting in the backlog is reported again on the very
//! next turn of the loop. The loop therefore has to leave the backlog empty
//! every time it runs: it accepts until `accept` says there is nothing left
//! (`WouldBlock`), and whatever else `accept` says decides what happens next.
//! Anything that leaves a pending connection behind *and* returns `Continue`
//! is a whole-compositor busy-spin -- the event loop wakes, fails, and goes
//! round immediately, at 100% CPU with nothing to show for it. That is the
//! crash/hang severity class: a starving compositor takes every client's
//! unsaved state with it just as surely as a panicking one.
//!
//! The realistic way in is fd exhaustion (`EMFILE`/`ENFILE`): the process --
//! or the machine -- is out of file descriptors, which IPC's own connection
//! cap cannot prevent, because every Wayland client, DRM device and `wl_shm`
//! pool draws from the same table. Of the two standard mitigations, this uses
//! the spare-fd trick and deliberately not a back-off timer:
//!
//! - The trick consumes the backlog entry (close the spare, accept the pending
//!   connection, drop it), so the level trigger clears. Each turn of the loop
//!   then either consumes one entry or leaves, which bounds the loop by the
//!   backlog's length -- no timer, no extra event source, no new shutdown
//!   ordering to get wrong.
//! - A back-off would leave the pending connection sitting in the backlog and
//!   delay *every* new connection for the length of the back-off, including an
//!   innocent client's once fds free up again. The trick serves that client
//!   with no added delay the moment pressure lifts.
//!
//! With no spare to spend -- never armed, or a previous shed's re-arm failed
//! (its freed fd was stolen in the window; see [`shed_one`]) -- the accept
//! is still attempted: pressure that has lifted meanwhile lets it succeed,
//! which re-arms the spare on the spot. If exhaustion persists *with* no
//! spare, the loop returns `Continue` with the backlog still pending, and the
//! level trigger re-reports it on the very next turn: that is a busy-spin,
//! not a stall -- each turn still yields to the event loop first, so every
//! other source is served and the compositor stays alive, and the error
//! logged per turn is the canary that names the cause. That per-turn error is
//! deliberately not rate-limited: this corner is already narrow (below), and
//! throttling the log would throttle the one trace a starving compositor
//! leaves. It recovers the instant fds free up. Reaching this corner takes
//! both halves at once -- a lost close-to-accept race *and* exhaustion that
//! never lifts -- and with the spare armed, which is the steady state, every
//! shed consumes one backlog entry, so the loop provably terminates.
//!
//! What the shed client sees is an immediate EOF with no refusal line. That is
//! exhaustion, not the connection cap, and the two stay distinguishable both
//! in the logs (this warns about file descriptors; the cap debugs about taken
//! slots) and on the wire (a cap refusal still gets its reason; here there is
//! no fd to serve even that with -- the only option is the drop).
//!
//! ## What may run in a forked child
//!
//! The exhaustion test (see `tests`) lowers `RLIMIT_NOFILE` in a *forked*
//! child -- the limit is process-global, so lowering it in-process would fail
//! unrelated tests running on other threads, while the child gets its own
//! copy and leaves the suite alone. A forked child of a multithreaded runner
//! may not allocate or take locks (another thread may hold them frozen), so
//! [`drain`], [`shed_one`] and [`classify`] -- and everything they call --
//! must stay allocation- and lock-free on the paths the child exercises. That
//! is why the shed-accepted socket *becomes* the new spare (see [`shed_one`])
//! instead of being dropped and reopened with `File::open` -- the latter
//! builds a `CString`, which allocates -- and why the one reopen this module
//! does (a shed that found no backlog, same function) is a raw `libc::open`
//! on a static path, the same call the test child's own fill loop uses.
//! Keep it that way: a `format!` or a `File::open` on this path turns the
//! exhaustion test from deterministic into hanging when the fork lands badly.
//! (`tracing`'s macros are fine -- with no subscriber installed, as in the
//! test, they evaluate nothing.)
//!
//! [`Spare`], [`classify`], [`Disposition`] and [`ShedOutcome`] are shared
//! with the Wayland listener (`super::super::wayland_accept`), which sheds
//! the same way over a different socket type: everything the sharing relies
//! on -- the take/put pair below, the classification, the outcome -- stays
//! under the same allocation- and lock-free discipline, for the same
//! forked-child reason.

use std::cell::Cell;
use std::fs::File;
use std::io::{self, ErrorKind};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};

use smithay::reexports::calloop::PostAction;

#[cfg(test)]
mod tests;

/// One file descriptor held open for the sole purpose of being spent.
///
/// Created once at startup, never read or written: when `accept` fails with
/// `EMFILE`/`ENFILE`, closing this frees exactly the one fd the next `accept`
/// needs to consume the pending connection out of the backlog. Kept armed by
/// [`shed_one`] -- repurposed from the shed connection, or raw-reopened when
/// the backlog raced away -- so the cost is one fd held for the session and
/// nothing per connection.
///
/// Shared with the Wayland listener: its shed spends and re-arms through
/// [`Spare::take`] / [`Spare::put`] rather than reimplementing the slot.
///
/// No `Debug`: `Cell<Option<File>>` is only `Debug` for `Copy` contents, and
/// a peek that takes the spare out and puts it back belongs in the test-only
/// [`Spare::is_armed`], not in a formatting impl.
pub(crate) struct Spare {
    slot: Cell<Option<File>>,
}

impl Spare {
    pub(crate) fn new() -> Self {
        let spare = File::open("/dev/null").ok();
        if spare.is_none() {
            // The mitigation below is disarmed from the start. Exhaustion is
            // still loud when it happens (see `drain`), but it will not shed.
            tracing::warn!(
                "no spare fd for the ipc accept loop (/dev/null would not open); \
                 fd exhaustion will be logged but not shed"
            );
        }
        Self {
            slot: Cell::new(spare),
        }
    }

    /// Whether a spare is currently held. Test-only: `Cell` has no peek and
    /// a `File` cannot be cloned without spending a new fd, so this takes the
    /// spare out and puts it straight back.
    #[cfg(test)]
    pub(crate) fn is_armed(&self) -> bool {
        let spare = self.slot.take();
        let armed = spare.is_some();
        self.slot.set(spare);
        armed
    }

    /// Spends the spare: the fd the next `accept` needs. The spender owns
    /// what comes back and must hand a replacement to [`Spare::put`] --
    /// the accepted socket's own fd, or a raw reopen when the backlog raced
    /// away. Allocation- and lock-free (a `Cell` take), so the Wayland shed
    /// can call it from its own forked exhaustion child.
    pub(crate) fn take(&self) -> Option<File> {
        self.slot.take()
    }

    /// Re-arms the spare with a replacement fd. Same discipline as
    /// [`Spare::take`]: a `Cell` set, nothing more.
    pub(crate) fn put(&self, spare: File) {
        self.slot.set(Some(spare));
    }
}

/// What an `accept` error means for the loop.
pub(crate) enum Disposition {
    /// `WouldBlock`/`Interrupted`: nothing to do -- leave, and the level
    /// trigger reports again if anything is still pending.
    Done,
    /// `EMFILE`/`ENFILE`: shed one pending connection via the spare fd.
    Shed,
    /// `EBADF`/`EINVAL`: the listener is dead; deregister the source.
    Dead,
    /// Anything else: log loudly and leave the listener registered.
    Other,
}

/// Sorts an `accept` error into what the loop should do about it.
///
/// Matched on the raw errno for everything but the two done-for-now kinds:
/// what `ErrorKind` groups together is not what this loop acts on (`EMFILE`
/// is not an allocation failure in any sense this loop could use), and the
/// exact constant is what the man page promises.
pub(crate) fn classify(error: &io::Error) -> Disposition {
    match error.kind() {
        ErrorKind::WouldBlock | ErrorKind::Interrupted => Disposition::Done,
        _ => match error.raw_os_error() {
            Some(code) if code == libc::EMFILE || code == libc::ENFILE => Disposition::Shed,
            Some(code) if code == libc::EBADF || code == libc::EINVAL => Disposition::Dead,
            _ => Disposition::Other,
        },
    }
}

/// What one mitigation attempt did.
pub(crate) enum ShedOutcome {
    /// A pending connection was accepted and dropped: the backlog shrank.
    Consumed,
    /// Nothing was pending after all (or a signal got in first): the earlier
    /// error raced a client going away. Quiet -- there is nothing to report.
    BacklogEmpty,
    /// No progress: still failing with the spare spent. Carries the error
    /// that refused to clear, so the caller can log what it actually was --
    /// every turn it recurs, which is the canary for the corner the module
    /// doc describes (deliberately not rate-limited: see there).
    Stuck(io::Error),
}

/// Spends the spare to consume one backlog entry: close it, accept the pending
/// connection, and keep that connection's fd as the new spare.
///
/// Repurposing rather than reopening is the whole point: the accepted socket
/// hands over exactly the fd the trick just spent, with no `open`, no path
/// lookup and no allocation -- so this stays runnable in the forked child the
/// exhaustion test uses (see the module doc), where allocating could deadlock
/// against another thread's frozen lock. A spare that was never armed changes
/// nothing: the accept is attempted anyway, and success re-arms from the
/// freed fd on the spot.
///
/// The one path that reopens is a shed that found no backlog: the spent spare
/// has to come back from somewhere, or the mitigation stays silently disarmed
/// and the *next* exhaustion goes `Stuck` instead of shedding. That reopen is
/// a raw `libc::open` on a static path -- the same call the test child's fill
/// loop uses -- never `File::open`, which would allocate.
pub(super) fn shed_one(listener: &UnixListener, spare: &Spare) -> ShedOutcome {
    // The whole trick: this close frees exactly one fd, which is what the
    // accept below spends. No code on this thread runs between the two -- but
    // the fd table is process-global, so another thread (since PR #57, the
    // screenshot worker does its own opens) can still steal it in that
    // window. Narrow, and self-resolving -- the next shed attempt succeeds
    // the moment pressure lifts and re-arms on the spot -- but not
    // impossible, which is the shape the `Stuck` corner has.
    drop(spare.slot.take());
    match listener.accept() {
        Ok((pending, _)) => {
            spare.slot.set(Some(File::from(OwnedFd::from(pending))));
            ShedOutcome::Consumed
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
            // Nothing came back for the spent spare, so re-arm straight away.
            // Raw `open`, not `File::open`: see above.
            let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
            if fd >= 0 {
                // SAFETY: `open` just returned this fd; it is open and owned.
                spare.slot.set(Some(unsafe { File::from_raw_fd(fd) }));
            } else {
                // The freed fd was stolen in the window (or `/dev/null`
                // would not open): disarmed, loudly rather than silently.
                tracing::warn!(
                    "could not re-arm the ipc accept loop's spare fd; \
                     further fd exhaustion will be logged but not shed"
                );
            }
            ShedOutcome::BacklogEmpty
        }
        Err(error) => ShedOutcome::Stuck(error),
    }
}

/// Accepts until the backlog is empty, shedding through the spare on fd
/// exhaustion. Returns what the event-loop callback should return with:
///
/// - `Continue`: the backlog is drained (or was never the problem), and the
///   level trigger has nothing left to re-report -- the no-spin guarantee.
/// - `Remove`: the listener itself is dead; waking into the same failure
///   forever would be the spin this module exists to prevent.
pub(super) fn drain(
    listener: &UnixListener,
    spare: &Spare,
    take: &mut impl FnMut(UnixStream),
) -> PostAction {
    loop {
        match listener.accept() {
            Ok((stream, _)) => take(stream),
            Err(error) => match classify(&error) {
                Disposition::Done => return PostAction::Continue,
                Disposition::Shed => match shed_one(listener, spare) {
                    ShedOutcome::Consumed => {
                        tracing::warn!(
                            "out of file descriptors; shed a pending ipc connection \
                             (exhaustion, not the connection cap: a refused-over-cap \
                             client still gets its reason, a shed one gets EOF)"
                        );
                    }
                    ShedOutcome::BacklogEmpty => return PostAction::Continue,
                    ShedOutcome::Stuck(error) => {
                        tracing::error!(
                            %error,
                            "ipc accept keeps failing with no fd to spend; \
                             leaving the backlog pending"
                        );
                        return PostAction::Continue;
                    }
                },
                Disposition::Dead => {
                    tracing::error!(
                        %error,
                        "ipc listener is dead; removing the accept source"
                    );
                    return PostAction::Remove;
                }
                Disposition::Other => {
                    tracing::error!(
                        %error,
                        "ipc accept failed; leaving the listener registered"
                    );
                    return PostAction::Continue;
                }
            },
        }
    }
}
