---
title: "Golden screenshots per shipped look from the headless harness"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-16"
---

# Golden screenshots per shipped look from the headless harness

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**computer use** (a look is a reproducible screenshot).

## The gap

The only per-look screenshot is a manual script
(`docs/examples/ginger-night/screenshot.sh`). The CI validate gate lives in
[`look-shipped`](../core/look-shipped.md); this ticket adds pixels.

## What to do

A headless harness test per shipped look with a private `XDG_STATE_HOME`
and `XDG_RUNTIME_DIR` (so it can never repoint a real desktop): `scoot
--headless` with fixed clients (a solid-color test client, the bar),
`scoot msg look-apply NAME`, `scoot msg screenshot`, compared with a
committed golden at a small per-channel tolerance; a documented regenerate
command.

## Evidence to record

The goldens, the tolerance and why, and one deliberately broken look shown
failing.

## Not in this ticket

Third-party apps in the screenshots (fonts and versions make them flaky).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (2026-10-16)

Landed as `test(scoot,ci): golden screenshots per shipped look` (PR #641):
`crates/scoot/tests/look_golden_screenshots.rs` (one `#[test]` per look,
the `every_shipped_look_has_a_golden` completeness gate, the
`a_broken_look_is_nothing_like_its_golden` negative control) with the
five goldens in `crates/scoot/tests/goldens/look-<name>.png`.

- Scene: `scoot --headless` (800x600, `--renderer cpu`) on a scratch
  tree (private `XDG_STATE_HOME`/`XDG_RUNTIME_DIR`, scratch home/config/
  cache/tmp, stub-only `PATH`, DejaVu-only `FONTCONFIG_FILE`), a magenta
  `xdg_toplevel` test client plus a real `scootbar daemon` with only the
  `workspaces` module, `scoot msg look-apply NAME`, then screenshots
  until two in a row are byte-identical.
- Tolerance 2 per channel: PNG is lossless, so the only rounding is
  blending (focus ring and rounded corners composite once over what is
  behind them, 1 LSB each). A wrong color misses by tens or more.
- Broken look (ginger-night with a white focus ring) misses by 3752
  pixels past tolerance, worst 207 in blue (`#FF9A30` to `#FFFFFF`).
- Regen is one look at a time (`SCOOT_GOLDEN_REGENERATE=1`, see the
  module doc); CI never regenerates, compares, and uploads
  `look-golden-actual` on failure. CI builds `-p scootbar` and sets
  `SCOOT_GOLDEN_REQUIRE_BAR` in the default `linux` test steps.
- Stub-only `PATH` keeps `scootbg` out of reach, so image-wallpaper
  looks render their flat background color; the bar's own binary is
  found beside the test binary, so its fragment check still runs.
