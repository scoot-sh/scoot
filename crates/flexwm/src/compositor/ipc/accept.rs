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
//! With no spare to spend (it was never armed, or a previous shed left it
//! disarmed -- see [`shed_one`]), the accept is still attempted: pressure that
//! has lifted meanwhile lets it succeed, which re-arms the spare on the spot.
//! Only an exhaustion that persists *with* no spare leaves the backlog
//! pending, and the loop still yields to the event loop every turn it cannot
//! make progress, so even that corner stalls rather than hangs, and recovers
//! the instant fds free up.
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
//! is why there is no reopen here: the shed-accepted socket *becomes* the new
//! spare (see [`shed_one`]), so no `File::open`, no path lookup, no `CString`
//! is ever needed past startup. Keep it that way: a `format!` or an `open` on
//! this path turns the exhaustion test from deterministic into hanging when
//! the fork lands badly. (`tracing`'s macros are fine -- with no subscriber
//! installed, as in the test, they evaluate nothing.)

use std::cell::Cell;
use std::fs::File;
use std::io::{self, ErrorKind};
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};

use smithay::reexports::calloop::PostAction;

#[cfg(test)]
mod tests;

/// One file descriptor held open for the sole purpose of being spent.
///
/// Created once at startup, never read or written: when `accept` fails with
/// `EMFILE`/`ENFILE`, closing this frees exactly the one fd the next `accept`
/// needs to consume the pending connection out of the backlog. Reopened
/// straight afterwards (see [`shed_one`]), so the cost is one fd held for the
/// session and nothing per connection.
pub(super) struct Spare {
    slot: Cell<Option<File>>,
}

impl Spare {
    pub(super) fn new() -> Self {
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
}

/// What an `accept` error means for the loop.
pub(super) enum Disposition {
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
pub(super) fn classify(error: &io::Error) -> Disposition {
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
pub(super) enum ShedOutcome {
    /// A pending connection was accepted and dropped: the backlog shrank.
    Consumed,
    /// Nothing was pending after all (or a signal got in first): the earlier
    /// error raced a client going away. Quiet -- there is nothing to report.
    BacklogEmpty,
    /// No progress: still failing with the spare spent. Carries the error
    /// that refused to clear, so the caller can log what it actually was.
    Stuck(io::Error),
}

/// Spends the spare to consume one backlog entry: close it, accept the pending
/// connection, and keep that connection's fd as the new spare.
///
/// Repurposing rather than reopening is the whole point: the accepted socket
/// hands over exactly the fd the trick just spent, with no `open`, no path
/// lookup and no allocation -- so this stays runnable in the forked child the
/// exhaustion test uses (see the module doc), where allocating could deadlock
/// against another thread's frozen lock. A spare that was never armed (or
/// that a previous shed left disarmed) changes nothing: the accept is
/// attempted anyway, and success re-arms from the freed fd on the spot.
pub(super) fn shed_one(listener: &UnixListener, spare: &Spare) -> ShedOutcome {
    // The whole trick: this close frees exactly one fd, which is what the
    // accept below spends. Single-threaded, so nothing interleaves between
    // the two and takes it first.
    drop(spare.slot.take());
    match listener.accept() {
        Ok((pending, _)) => {
            spare.slot.set(Some(File::from(OwnedFd::from(pending))));
            ShedOutcome::Consumed
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
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
