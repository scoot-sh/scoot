---
title: "scootui Claim chmods the control socket after bind; scootbg duplicate too"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scootui Claim chmods the control socket after bind; scootbg duplicate too

Filed 2026-10-27. Serves **daily-drive** (a world-writable control socket lets another local user drive the bar/notify daemon; daily use must not depend on umask or a loose runtime dir).

## The gap

`crates/scootui/src/control/claim.rs:96-106` (`Claim::acquire`) binds the control socket (`scootbar-<display>.sock`, `scootnotify-<display>.sock` via `crates/scootui/src/control/paths.rs`) and only then chmods it to `0600`:

```rust
let listener = UnixListener::bind(&paths.socket)...?;
fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))...?;
```

Between the two calls the socket exists with `0777 & !umask` (e.g. `0755` under the common `022`, `0777` under `000`), connectable by anyone who can traverse the directory. `$XDG_RUNTIME_DIR` is normally `0700`, which closes the traversal, but the file mode is still wrong until the chmod lands, and a loose directory mode (or a moved socket, which the code's own comment anticipates: "if the file is ever moved or the dir's mode is loose") leaves a window where another uid can connect. `SO_PEERCRED` uid checks in `crates/scootui/src/control/mod.rs` mitigate (non-owner peers are ignored) but do not stop root, and the window is still a wrong-mode publication. `crates/scootbg/src/control/claim.rs:88-106` duplicates the same bind-then-chmod shape for `scootbg`.

Contrast `crates/scoot/src/compositor/ipc/listener.rs:91-179`, which builds the socket inside a freshly-created `0700` staging directory, chmods it `0600`, and renames it onto the published name atomically, so the published name is only ever the previous socket or a `0600` one. `scootlaunch` now does the same (`crates/scootlaunch/src/socket.rs:bind_staged`, fixed in `launch-toggle-race-and-socket-mode`).

## What to do

Make `scootui::control::Claim::acquire` (and the `scootbg` duplicate, or de-duplicate them) publish the socket `0600` with no chmod-after-bind window: either the staging-directory + atomic rename from `scoot/src/compositor/ipc/listener.rs`, or a `umask 077`-style mask held across the bind (process-wide, so only before threads exist), never a post-bind chmod. Pin with a test that stats the published socket mode (`0o600 & 0o777`) under a permissive umask (e.g. `022` and `000`), plus the existing stale/non-socket refusal cases. Keep the lock-file claim as is (the kernel drops the flock on crash; the lock file itself is never removed).

## Not in this ticket

The `scootlaunch` toggle fix itself (atomic lock + `0600`-before-publish): that lands in `launch-toggle-race-and-socket-mode`. This ticket is the shared control-socket pattern only.
