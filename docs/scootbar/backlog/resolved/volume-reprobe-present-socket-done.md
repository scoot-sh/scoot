---
title: "Volume: re-probe a present socket after drop/refusal"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
---

# Volume: re-probe a present socket after drop/refusal

Filed 2026-10-02 from the volume-module review (PR #376). Serves
**daily-drive**: a volume module stuck `Unavailable` while the server runs
is a dead control next to the user's media keys.

## The gap

`drop_live` re-arms the inotify watch but never probes the socket, so the
module only reconnects on a *future* event naming `native`. Two strandings:

- **Fast restart**: the server drops the connection but the socket path
  never disappears (restart keeps the path, or the watch re-arms after it
  reappeared). No new `CREATE`/`MOVED_TO` arrives; the module sits
  `Unavailable` beside a live server.
- **Refused handshake**: an auth refusal drops back to the same watch, and
  likewise waits for an event that may never come.

The naive fix (probe on every arm) spins: a server that persistently
refuses (wrong cookie against real PulseAudio) would connect-fail on every
HUP/ERR/NVAL re-arm. The fix needs refusal awareness — e.g. probe once per
arm but remember a refusal until a genuinely new event arrives.

## What to do

After `watch()` re-arms with no live connection, probe the socket when it
may already be present, without turning a standing refusal into a connect
loop. Pin both scenarios in the harness: drop-with-socket-present
reconnects, standing-refusal stays quiet. Measure idle wakeups: still zero
with no audio activity.

## Not in this ticket

The scan test that guards the watch path — see
[volume-scan-names-test](volume-scan-names-test-done.md).

## What landed

Commit `a57fb4cd1`. `drop_live` probes the socket once, after `watch()` has
re-armed (probing first could miss an appearance in between), when the
dropped connection had reached the subscription. The refusal awareness is
stateless: a connection dropped before that (an error to `AUTH`, a close during
the handshake) is not probed, so a standing refusal waits for a real directory
event instead of looping; the next `CREATE`/`MOVED_TO` of `native` ends it. The
directory watch also probes after an error re-arm and after following the watch
into a directory that appeared, both places a present socket was missed.
Residual: a server that completes the whole handshake and then drops, every
time, reconnects once per drop, paced by its own handshake round trips.

Pinned in `modules/volume/tests.rs`: `a_drop_with_the_socket_still_present_reconnects`,
`a_standing_refusal_is_not_a_connect_loop` (error to AUTH, then quiet, with
no unaccepted connection queued), `a_server_that_closes_during_the_handshake_is_not_a_connect_loop`
and `a_refusal_ends_at_the_next_event_on_the_directory`. Mutation check on
the VM: never probing times out the reconnect test; always probing fails both
no-loop tests.

Idle wakeups (dev VM, test thread, `/proc/thread-self/status`, a 200 to 500 ms
idle window after each scenario): `poll returns=0` in every window, and the
voluntary context switches rose by exactly 1 (the poll's own timeout sleep):
`500ms: 1 -> 2`, `200ms: 0 -> 1`, `500ms: 0 -> 1`, `500ms: 0 -> 1`. The
`/proc` reading was a temporary local instrumentation, not committed; the
committed tests assert zero poll returns. Same verification run as
[volume-scan-names-test](volume-scan-names-test-done.md): 793 passed, clippy and fmt
clean.
