---
title: "The compositor reads a look layer under the user's config, refused whole when bad"
status: "open"
area: "core"
priority: "high"
blocked: "look-format"
---

# The compositor reads a look layer under the user's config, refused whole when bad

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the compositor's ring, background, cursor and wallpaper are the most
visible half of a look, and the screenshots agents take.

## The gap

scoot reads one file (`config::reload_from`, `config.rs:756-767`) with no
way to layer a look under it. Inside a file that parses, a bad color falls
back to the default per field, at startup and at reload alike
(`config.rs:194-206,748-753`), so a look with one typo would be silently
half-applied. A ring or background change also runs `apply()`, which
re-arranges every window (`reload.rs:308-321`, `shell.rs:375-405`), though
neither is placement.

## What to do

- Read the look layer from `$XDG_STATE_HOME/scoot/look/scoot.toml` (the
  design doc's §4.1), merged *under* the user's file per key (user > look >
  default). A user opt-out key (for example `[look] layer = false`) for
  anyone who wants their file alone.
- The layer is parsed strictly: an unknown or non-allowlisted key (any
  command, `[binds]`, `[autostart]`, `[wallpaper] command`, the restart-only
  sections of `reload.rs:77-90`) or one bad color refuses the whole layer by
  name. At startup that means "no look layer" plus an error log (startup
  never fails over a look, the rule at `config.rs:16-23`); at reload it means
  the reload refuses and the running session keeps everything.
- Wallpaper paths in the layer are absolute (the renderer resolves them), so
  the relative-path rule for the user's own file is untouched.
- A ring/background/corner-only change requests a redraw without `apply()`,
  the way cursor fields already do (`reload.rs:322-331`); measure first.
- A reload option the swap can wait on: the reply comes after the wallpaper
  run finishes, not merely after it is handed over (`reload.rs:125-133`,
  `wallpaper/queue.rs:1-37`).
- Docs in the same PR: the config reference on the site gains the look
  layer and its precedence.

## Evidence to record

Tests: layer precedence per key, a strict-refusal test per forbidden key, a
bad-color layer refused whole at reload and dropped at startup, a dangling
link at startup. Benchmark before and after on headless with 1, 10 and 50
windows: the time of a reload that changes only ring colors (with and
without `apply()`), and frame times across it.

## Not in this ticket

The CLI that writes the link, scootbar, IPC.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
