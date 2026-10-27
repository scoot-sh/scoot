---
title: "Reload wallpaper wait as a deferred reply: park the IPC reply, answer from the reaper or a timer"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# Reload wallpaper wait as a deferred reply: park the IPC reply, answer from the reaper or a timer

Filed 2026-10-15, split out of the look-layer-scoot review (#613). Serves
**daily-drive**: a user or agent that asks "did my wallpaper land" needs an
answer without freezing the session to get it.

## The gap

`scoot msg reload --wait-wallpaper` blocked the event-loop thread with
`thread::sleep` polling (`reload.rs::wait_wallpaper`), freezing the entire
compositor for up to 30 s -- and it always timed out, because scootbg needs
the loop to service its Wayland requests while it works. The flag was
removed from the protocol in #613 (`Request::Reload` is a unit variant
again); the reload reply answers once the wallpaper run is handed over,
without waiting for it. There is no way to ask "tell me when the wallpaper
run exits".

## What to do

Design a deferred reply: park the IPC reply on the reload, answer it from
the `SIGCHLD` reaper when the `scootbg apply-config` run exits (or from a
timer past a bound, reporting handed-over-but-unwaited-on). Pin the edge
cases: the run exits before the park, the client disconnects while parked,
two reloads park two replies, SIGHUP has no reply channel so it never parks.
Never block the loop thread: the park is state plus a wakeup, not a sleep.

## Not in this ticket

Changing what a plain reload applies, or the fire-and-forget default.

## Resolution

Landed `scoot msg reload --wait-wallpaper` as a deferred reply: new
distinct request tag `reload_wait_wallpaper` (`Request::ReloadWaitWallpaper`)
runs the plain reload then parks `PendingReloadWait{waited_seq}` until the
`apply-config` run exits (reaper answers when reaped seq >= waited seq) or
30 s passes (extra `wallpaper (timed out...)` refused entry). Plain `Reload`
never touches the queue (no allocation on the common path); two waits park
two replies; SIGHUP runs plain and never parks; disconnect drops on write
failure; superseded waits follow the newest.

Protocol: additive tag + existing `Reloaded` reply, so `PROTOCOL_VERSION`
stays 13; an older server answers unknown-tag `error`, never a silent
immediate (pinned in `scoot-ipc/tests/wire.rs`, same rationale as
`reload_look`).

Evidence: `cargo test -p scoot --bin scoot reload_wait` 7/7 with real IPC
clients (no-wallpaper immediate, parks-until-exit with concurrent `version`
live, two parks, disconnect drop, plain never parks, 2-output, hung-run
timeout); `cargo test -p scoot-ipc` 66 passed; `cargo test -p scootctl`
passed; `cargo nextest run -p scoot` 2884 passed; `cargo test -p scoot`
2841 passed +1 load flake (`subsurface_depth...deep`) which passes isolated;
`clippy -D warnings` clean; `fmt --check` clean. Live headless 2-output:
plain 27 ms (fire-and-forget), wait 4010 ms (two sequential 2 s stub runs),
concurrent `version` 42 ms during the park (old blocking poll would have
frozen the loop; old flag always timed out at 30 s).

PR: #769.
