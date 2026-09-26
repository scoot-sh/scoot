---
title: "The crate, the daemon and the control socket"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# The crate, the daemon and the control socket

The first real PR, and the one that turns the directory into a crate.

- Add `crates/scootbg` as a workspace member (drop the `exclude` entry in
  the root `Cargo.toml`), MIT, `description` saying "wallpaper daemon for
  Wayland" (not "compositor": it is a client).
- One binary with subcommands: `daemon` runs the Wayland client, and every
  other subcommand is a client of its control socket.
- Control socket at `$XDG_RUNTIME_DIR/scootbg-$WAYLAND_DISPLAY.sock`, so
  two sessions (a nested scoot inside another) never share one.
  `WAYLAND_DISPLAY` may be an absolute path, which libwayland allows, so
  derive the name from its final component (or a hash) rather than
  splicing it in. Line-framed
  JSON requests and replies, versioned from day one like `scoot-ipc`.
  Whether to reuse `scoot-ipc`'s framing code or keep scootbg's protocol
  self-contained is a decision for this PR: reuse only if it does not
  pull compositor types into a client that must build for any compositor.
- A second `scootbg daemon` on the same display refuses loudly (socket
  already live) instead of stacking a second set of surfaces; a stale
  socket from a crashed daemon is detected and replaced.
- A Wayland disconnect (compositor exit) ends the daemon cleanly with a
  non-zero status, never a panic.
- Nix: a `scootbg` package output of its own (the `scoot` package stays
  `scoot` only, as with `scootctl`) and a place in the dev shell; CI builds
  and tests it with the rest of the workspace.

Done when `scootbg daemon` connects, binds its globals, answers `query`
with an empty output list shape, and exits cleanly on `kill` and on
compositor exit, with tests for the socket lifecycle.
