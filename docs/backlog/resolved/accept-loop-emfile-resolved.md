---
title: "The IPC accept loop swallows `EMFILE` and can spin the event loop at 100% (LOW) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The IPC accept loop swallows `EMFILE` and can spin the event loop at 100% (LOW) — RESOLVED

## What it said

Filed 2026-09-16, out of the connection-cap review. `ipc.rs`'s accept
callback was `while let Ok((stream, _)) = listener.accept()`, which leaves
the loop on *any* error -- not just the `WouldBlock` that means "no more
connections pending" -- and says nothing about it. The listener is registered
`Mode::Level`, so a connection still sitting in the backlog is reported again
on the very next turn of the loop, and if `accept` keeps failing for a reason
that will not clear on its own, the event loop takes that wakeup, fails
again, and goes round immediately: 100% CPU with nothing to show for it. The
realistic way in is `EMFILE`/`ENFILE`, reachable via Wayland clients, DRM and
shm pools, not just IPC -- so the 64-connection cap makes it less reachable,
not unreachable. The sketch named the two standard mitigations (spare-fd
trick, or disable/back-off the listener) and asked for a test that exhausts
`RLIMIT_NOFILE` and asserts the loop does not spin.

## Resolution (2026-09-17)

**Spare-fd trick, no back-off** (`compositor/ipc/accept.rs`, new module; PR
#66). The decision, with reasons: the trick consumes the backlog entry (close
the spare, accept the pending connection, keep its fd as the new spare), so
the level trigger clears -- each turn of the loop either consumes one entry
or leaves, which bounds the loop by the backlog's length with no timer, no
extra event source and no new shutdown ordering to get wrong. A back-off
would leave the pending connection sitting in the backlog and delay *every*
new connection for its duration, including an innocent client's once fds free
up again; the trick serves that client with no added delay the moment
pressure lifts. No observable connection delay under load, so no README
change (a log line is not user-facing).

Error handling around it, per the ticket: `WouldBlock`/`Interrupted` end the
loop quietly as before; anything else is logged at an audible level
(`warn` for a shed connection, `error` for the rest). Exhaustion and
cap-full stay distinguishable both in the logs (fd-exhaustion `warn` vs the
cap's slot-taken `debug`) and on the wire (a cap refusal still gets its
reason; a shed connection gets an immediate EOF -- there is no fd to serve
even the refusal with). A dead listener (`EBADF`/`EINVAL`) deregisters the
source rather than waking into the same failure forever; anything else
 unexpected is logged loudly but leaves the listener registered, so a
 transient error cannot cost the whole control socket.

 Independent review found two things pre-merge, both fixed on the PR branch:

 - A shed that found no backlog (`WouldBlock`/`Interrupted` on the inner
   accept -- the outer `EMFILE` raced a disconnect) spent the spare and never
   re-armed it, silently disarming the mitigation until the next exhaustion
   went `Stuck`. The inner path now re-arms with a raw `libc::open` (never
   `File::open`, which would allocate and break the fork-child test), warns
   loudly if that fails, and is pinned by
   `a_shed_that_finds_no_backlog_re_arms_the_spare`.
 - The `Stuck` corner (persistent exhaustion with no spare) was misdescribed
   as stalling rather than spinning: with the backlog pending under a level
   trigger it busy-spins, and the per-turn error log -- deliberately not
   rate-limited -- is the canary. The doc now says so, narrows the
   close-to-accept steal window honestly (process-global table, reachable by
   any thread doing `open`, e.g. PR #57's worker), and notes the re-arm above
   is what makes the corner genuinely narrow.

Two things found while testing, both recorded in the code rather than fixed
around:

- `EMFILE`'s `ErrorKind` is `Uncategorized`, not anything the name suggests --
  which is why the mapping matches raw errnos, not kinds.
- `shutdown()` on a listening unix socket does *not* break `accept` (measured:
  it still accepts), so it is not a way to simulate a dead listener. The
  behavioral dead-listener test drives `EINVAL` through a socket that never
  listened instead; `EBADF` reaches the same match arm by construction and is
  covered per-errno in the mapping tests (closing a live fd would leave a
  window where another thread's new fd reuses the number mid-test).

**Tests** (`accept/tests.rs`, own module): error-kind unit coverage
(WouldBlock/Interrupted end cleanly, EMFILE/ENFILE shed, EBADF/EINVAL
deregister, anything else stays registered); loop behavior against a live
listener (idle ends clean, pending connections are served then it ends, dead
listener is removed); and the required exhaustion test, which lowers
`RLIMIT_NOFILE` and asserts the loop returns `Continue`, sheds both pending
clients with EOF and no refusal line, leaves `WouldBlock` behind it (the
no-spin proof -- that is what clears a level trigger), and serves a fresh
connection immediately after.

The exhaustion test runs its scenario in a **forked child**, not in-process:
the limit is process-global, and lowering it in-process failed unrelated
tests on other threads while this was being written (parallel closes freed
slots and an accept "succeeded while exhausted" -- the fail-first probe is
kept in the child, where it is deterministic). The child inherits copies of
every fd and its own copy of the limit, so the suite needs no restore step
and cannot tell the test ran. Discipline, stated in the module doc because a
forked child of a multithreaded runner may not allocate or take locks: the
drain path is allocation- and lock-free by construction (the shed-accepted
socket *becomes* the new spare, so no `File::open` past startup), the child
uses raw `libc` calls with stack buffers, and it reports by exit status
rather than panicking.

**Verified**: `cargo test -p flexwm` (720 passed), `cargo nextest run
--workspace` (819 passed), `cargo clippy -p flexwm --all-targets -- -D
warnings` clean, `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh`
exit 0 (15 oks). No benchmark: the success path is instruction-identical to
the old loop (`Ok` goes straight to `take`; the new matching runs only on
errors, which are cold), and the per-request hot path is untouched.
