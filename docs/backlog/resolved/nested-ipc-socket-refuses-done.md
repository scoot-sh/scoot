---
title: "Nested scoot in the Selkies webtop image stops accepting IPC connections after the first query"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Nested scoot in the Selkies webtop image stops accepting IPC connections after the first query

Filed 2026-10-05 from the nixos-webtop VNC benchmark (yackey-labs/nixos-webtop#30).
Serves **computer use** (the webtop container is the agent's machine, and
IPC is how an agent drives it) and **daily-drive** (the bar and the clipboard
guard talk to scoot over IPC too).

## The gap

Reproduced twice in the `selkies-nix-webtop-scoot` image on the Asahi M2
(Docker, scoot `3ff4c5a2a`, scoot running `--nested` inside pixelflux's
compositor):

- One `scoot msg` query per container boot succeeded. Every later
  connection got `Connection refused`.
- `ss -xln` inside the container showed no listener on
  `/run/user/911/scoot.sock`, while the compositor kept serving Wayland
  (`wayland-2` listening, the bar mapped).
- The logs showed `Io error: Broken pipe` and `other error during loop
  operation`.
- Headless scoot at the same rev, in the sibling `scoot-vnc` image, served
  dozens of clients without a problem.

Not yet established, so measure before fixing:

- Whether the accept source was removed (the `Disposition::Dead` path in
  `crates/scoot/src/compositor/ipc/accept.rs` logs "ipc listener is dead").
- Whether the socket file was unlinked or replaced by another process. A
  second scoot, the session script, or a `SCOOT_SOCKET`/`XDG_RUNTIME_DIR`
  mismatch between the driver and the compositor are all candidates.
- Whether an event-loop dispatch error ("other error during loop
  operation") tore something down.

`Connection refused` with no listener is consistent with an unlinked path,
not only with a dead fd.

## What to do

- Reproduce outside the image first: `scoot --nested` under a host
  compositor (headless scoot, sway or cage) on the M2. Repeat `scoot msg`
  queries, including clients that disconnect before the reply (the
  broken-pipe shape). Then reproduce in the Selkies image if that doesn't
  show it.
- Find which of the candidates above it is, with logs at debug level and
  `ls -li`/`ss -xlp` on the socket before and after.
- Fix at that layer, with a regression test that fails before the fix.
  If the cause is outside scoot (the image's session scripts), fix it in
  nixos-webtop and say so here.

## Not in this ticket

- The VNC image's fixed output size (no output-management protocol for wayvnc).

## Resolution (2026-10-06)

**Mechanism, measured on the Asahi M2 at `7ed903f7`:** the nested scoot
process had exited when its host connection broke, leaving a stale
`scoot.sock` file with no listener behind -- which is exactly
`Connection refused` with nothing in `ss -xln`. The accept source was
exonerated (zero `ipc listener is dead` lines; the listener served
queries until the death), and so were the image scripts (a single
`exec scoot`, no socket cleanup, no second starter, no env mismatch --
nothing to fix in nixos-webtop).

The chain, each link verified against the pinned sources: the host
dies, the next read/flush on the host connection fails with EPIPE,
`wayland-backend` prints the bare `Io error: Broken pipe`, the
`WaylandSource` error propagates out of `process_events`, calloop's
dispatcher wraps it in `Error::OtherError`, `EventLoop::run` returns it,
and the process exits on `scoot: other error during loop operation:
underlying IO error: Broken pipe` -- both of the report's log strings,
reproduced verbatim by killing a headless host under a nested scoot.
Rude IPC clients (500 sequential plus thousands of concurrent early-close
connects) never reproduced it; the only refusals seen were the by-design
64-connection slot-cap refusals, with the listener intact.

**Fix:** the host connection is now a `HostSource` (`nested.rs`) that
maps every host-connection failure to a loud error naming the host
(`lost the connection to the host compositor; stopping the session`),
a clean loop stop (the same treatment a failed server-side Wayland
dispatch already gets), and source removal; `compositor::run` turns
the stop into exit 1 with the same message. Deliberately no
unlink-the-stale-socket on exit: a second scoot that rebound the path
in between would lose its live socket file to the first one's cleanup.
Deliberately no survive-and-reconnect either: without the host nothing
presents and no input arrives, and a lingering process would hide the
outage from the supervisor that restarts it.

**Regression test:**
`compositor::nested::tests::losing_the_host_stops_the_session_without_failing_the_loop`
(a headless host harness on a thread, dropped mid-session; the nested
dispatch must stay `Ok`, the loss flag must set, the log must name the
host and contain no `other error during loop operation`). Fails before
the fix (`Err(IoError(BrokenPipe))` out of dispatch, exit 101), passes
after.
