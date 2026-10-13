---
title: "The compositor reads a look layer under the user's config, with a look-scoped reload"
status: "open"
area: "core"
priority: "high"
blocked: "look-format"
---

# The compositor reads a look layer under the user's config, with a look-scoped reload

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the ring, background, cursor and wallpaper are the most visible half of a
look, and the screenshots agents take.

## The gap

scoot reads one file (`config::reload_from`, `config.rs:756-767`). A bad
color in it falls back to `Appearance::default()` (`config.rs:194-206`),
which under a look layer would show neither the user's color nor the look's.
A reload re-reads everything, so a swap would also apply the user's pending
edits, autostart included (`reload.rs:101-112`). Ring and background changes
run `apply()` (`reload.rs:308-321`). `[wallpaper]` is fire-and-forget
(`reload.rs:125-133`).

## What to do

Per the design doc's sections 4.1 to 4.5:

- Read `<state>/look/scoot.toml` (`<state>` resolved as `--state`,
  `$SCOOT_LOOK_STATE`, `$XDG_STATE_HOME/scoot`, `~/.local/state/scoot`) and
  merge it under the user's file **on raw keys, before derivation**, with
  the units merged whole: `[wallpaper]` with its per-output and
  per-workspace tables (the color+image trap,
  `docs/backlog/resolved/stylix-wallpaper-color-trap-done.md:17-24`), the
  ring-width pair, the cursor triple.
- The layer is parsed strictly against scoot's allowlist from `scoot-look`:
  a forbidden key or a bad value refuses the whole layer (startup: no layer,
  an error log; reload: refused, everything kept).
- A bad *user* value for a key the layer provides falls back to the layer's
  value, and the reply reports it as invalid and shadowing.
- Keep a snapshot of the user file's raw table at every successful load
  (new state: today the reload diffs a fresh `LoadedConfig` against live
  `State` fields, `reload.rs:291-301`, and keeps no raw table).
  `Request::Reload` gains a scope: `look` merges the new layer under that
  snapshot and swaps only the layer (no autostart, no binds, no pending
  edits).
- The reply lists shadowed keys and gains an option to wait for the
  wallpaper run.
- Emit `EventKind::Look` when a reload sees the layer's identity change (the
  rendered look's hash in its `render.json`; not `<state>/look.json`, which
  is the tool's record of the live choice).
- Export `SCOOT_LOOK_STATE` to children: the user state on `--tty`, a
  private per-display state under `$XDG_RUNTIME_DIR` on `--headless` and
  `--nested`, seeded from the user's choice (D10).
- Ring, background and corner changes request a redraw without `apply()`
  (measure first).
- Measure the cursor rebuild on the loop thread in a release build at 24,
  48 and 256 px (PR #572 reports 36.4 ms at 16 px and 1.82 s at 256 px in a
  debug build); if it exceeds a frame at common sizes, build the bitmaps on a
  worker and swap them in.
- Docs in the same PR: the config reference gains the look layer and its
  precedence.

## Evidence to record

Tests: precedence per key; each unit merged whole (a user `color` with a
look `image` yields the user's section); a forbidden key and a bad value
refused whole; a bad user color falling back to the layer; a scoped reload
not spawning a new autostart entry nor applying a pending bind; a dangling
link at startup; the event fired once per identity change; a headless
session not touching the user state. Benchmarks before/after on headless
with 1, 10 and 50 windows: a ring-only reload with and without `apply()`,
frame times across it, and the cursor rebuild times.

## Not in this ticket

The CLI, scootbar, IPC requests.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
