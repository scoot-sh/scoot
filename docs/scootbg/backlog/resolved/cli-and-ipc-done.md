---
title: "The CLI and control protocol"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# The CLI and control protocol

Requests for v1: `set`, `clear`, `query`, `kill`, `version`, and
`apply-config`, the one command a compositor's config drives scootbg
through (see [scoot-integration.md](scoot-integration.md)): it takes the
whole section as JSON, starts the daemon when none answers, and only
changes the wallpaper when the section itself changed.

- `set <path|#color>` is the one command for changing the wallpaper. An
  argument starting with `#` is a color (`#rrggbb`), anything else a path.
  `--output NAME` limits it to one output; `--mode` and `--fill` apply to
  images.

- `query` answers per output: name, size, scale, and what it shows (path,
  mode, color), as JSON, so an agent can check its work.
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

## Already there, from solid-color-done.md

[Ticket 4](resolved/solid-color-done.md) landed part of this, so what is
left here is smaller:

- **Done:** `set '#rrggbb' [--output NAME]` and `clear [--output NAME]`
  (CLI, `--help`, protocol 1 requests), unknown-output errors that change
  nothing, `query`'s `shows` (`{"color":...}`), and the reply after the
  commits and a `wl_display.sync` round trip, without blocking the loop
  (`crates/scootbg/src/waiters.rs`). The README documents all of it, with
  exit codes.
- **Left:** `set PATH`, `--mode`, `--fill`, `apply-config`, `shows` for
  images (an object, so it gains keys, e.g. `{"image":...,"mode":...}`),
  the "file not found / not an image / too large" errors, and the
  `./#name.png` note. The CLI refuses a path today with "images come in a
  later version" (exit 2), so the parser change is to accept it rather
  than to add a new command.
