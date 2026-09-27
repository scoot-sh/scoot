---
title: "The CLI and control protocol"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# The CLI and control protocol — RESOLVED

Resolved 2026-09-27, across two tickets, with one request moved on:
[Resolution](#resolution) at the end says where each item landed. The
original ticket follows unchanged.

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

## Resolution

Nothing is left here. Each item, and where it went:

| Item | Where |
|---|---|
| `set '#rrggbb'`, `clear`, `--output NAME`, `query`, `kill`, `version` | [ticket 4, solid-color-done.md](solid-color-done.md#what-landed) (the requests since [ticket 2](crate-and-daemon-done.md)) |
| `set PATH`, `--mode`, `--fill` (and `--filter`); an argument starting with `#` a color, anything else a path, made absolute by the CLI | [ticket 6, images-decode-and-fit-done.md](images-decode-and-fit-done.md#what-landed) |
| `query`'s `shows` for images: `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`, an object as planned, additive within protocol 1 | ticket 6 |
| Errors: file not found, not an image, image too large, unknown output; a failed decode leaves the previous wallpaper | unknown output in ticket 4; the image ones in ticket 6 (also: not a regular file, truncated or corrupt, too many images waiting) |
| `set` replies after the commits and a `wl_display.sync` round trip, without blocking the loop | ticket 4 (`crates/scootbg/src/waiters.rs`); images reuse it once decoded (ticket 6) |
| The `./#name.png` note | ticket 6, in `scootbg set --help`, the root README and [docs/scootbg/README.md](../../README.md) |
| `--help` for every subcommand; the README's command list | tickets 2, 4 and 6, each in its own PR |
| **`apply-config`** | **moved to [scoot-integration.md](../scoot-integration.md) (ticket 10).** It exists to carry scoot's `[wallpaper]` section, and its "only changes the wallpaper when the section itself changed" needs the saved state and fingerprint of [restore-state.md](../restore-state.md) (ticket 9), so it lands with the integration, not before. The CLI does not parse or advertise it until then. |
