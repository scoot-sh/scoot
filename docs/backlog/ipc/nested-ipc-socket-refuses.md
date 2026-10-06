---
title: "Nested scoot in the Selkies webtop image stops accepting IPC connections after the first query"
status: "open"
area: "ipc"
priority: "high"
blocked: null
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
