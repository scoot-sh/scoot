---
title: "The CLI and control protocol"
status: "open"
area: "scootbg"
priority: "high"
blocked: "needs the socket from crate-and-daemon.md"
---

# The CLI and control protocol

Requests for v1: `set`, `clear`, `query`, `kill`, `version`.

- `set <path|#colour>` is the one command for changing the wallpaper. An
  argument starting with `#` is a colour (`#rrggbb`), anything else a path.
  `--output NAME` limits it to one output; `--mode` and `--fill` apply to
  images.

- `query` answers per output: name, size, scale, and what it shows (path,
  mode, colour), as JSON, so an agent can check its work.
- Replies are `ok` or an error with a reason (file not found, not an image,
  image too large, unknown output). A failed decode leaves the previous
  wallpaper in place.
- `set` returns once the new buffer is committed on every targeted output
  *and* a `wl_display.sync` round trip after the commit has come back, so
  the compositor has processed the commit before the reply and a script can
  take a screenshot straight after. Never called synchronously from inside
  the compositor: see [scoot-integration.md](scoot-integration.md).
- A file whose name starts with `#` is given as `./#name.png`; the README
  says so.
- `--help` for every subcommand; the README's command list is updated in
  the same PR as any change to it.
