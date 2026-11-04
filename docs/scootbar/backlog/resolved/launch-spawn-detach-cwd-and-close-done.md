---
title: "scootlaunch: detach helper cwd check; watch the toggle during spawn"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-04"
---

# scootlaunch: detach helper cwd check; watch the toggle during spawn

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

Found reviewing #676, two nits. Both re-checked against `main` at 737cec6cf
(2026-11-04) and neither is a defect:

- **N1, the detach helper applies a relative `Path=` unchecked: already
  handled.** The helper never sees `Path=`. `launch_exec_report`
  (`crates/scootlaunch/src/spawn.rs`) calls `resolve_cwd` once, before
  choosing direct or detached, and that returns only an absolute existing
  directory (else `$HOME` under the same test, else `None`). Both
  `spawn_direct` and the `DetachSpec.cwd` get that resolved value, so the
  helper's `current_dir` is always absolute. `resolve_cwd` already pins the
  relative case (`spawn/tests.rs`); this PR adds the end-to-end pin for the
  detached path, `a_relative_path_key_starts_the_detached_app_in_home`
  (`tests/spawn.rs`), which was the only untested leg.
- **N2, the toggle-close is not watched during the spawn/scope step: not
  worth doing.** By the time the scope step runs, `spawn_direct` has already
  started the app, so a `close` cannot undo the launch; watching for it could
  only end the panel sooner. Ending sooner would abandon the scope move, which
  leaves the app in the launcher's cgroup, a worse outcome than the 500 ms
  (`SCOPE_TIMEOUT`) it saves. A second invocation's `close` sender
  (`socket.rs`) connects and exits without waiting, so nothing blocks on the
  panel. The wait is bounded and only on a slow bus. A close arriving then is
  dropped with the panel, which is already leaving.

## What to do

Nothing further. Re-open if a user reports a second bind press feeling
unresponsive during a launch on a slow bus; the fix would be to shorten
`SCOPE_TIMEOUT`, not to poll.

## Not in this ticket

Threads, or an async D-Bus call, for the scope step.

## Docs

No user-visible change; no site docs.
