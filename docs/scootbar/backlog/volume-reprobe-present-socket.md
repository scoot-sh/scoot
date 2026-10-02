---
title: "Volume: re-probe a present socket after drop/refusal"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M5"
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
[volume-scan-names-test](volume-scan-names-test.md).
