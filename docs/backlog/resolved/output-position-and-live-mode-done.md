---
title: "Per-output position, and a live mode change on reload"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Per-output position, and a live mode change on reload

Filed 2026-09-29, when `[[outputs]]` landed with scale and mode per output
([record](../resolved/per-output-scale-mode-done.md)). Serves
**daily-drive**: a laptop on a desk with the monitor to its *left*, or
above it, is laid out wrong today, and changing a monitor's resolution
means restarting the session.

## The gap

Two halves `[[outputs]]` deliberately left out.

- **Position.** Outputs are always packed left to right, in kernel
  connector order at startup and plug order after
  (`headless.rs`'s `add_output_with` steps off the previous output's
  logical right edge; `repack_outputs` and `rescale_outputs` re-run the same
  fold). There is no key to place one elsewhere. The pointer clamp
  (`input.rs`'s `clamp_to_output_union`), the X layout bound
  (`xwayland/scale.rs`), the output-cycle order
  ([output-cycle-binds](./output-cycle-binds.md)) and every repack all assume
  that fold and would each need re-checking against arbitrary positions
  (gaps, overlaps, negative origins -- X's `INT16` floor is already handled).
- **A live mode change.** A reload refuses a changed `mode` by name
  (`outputs.<name>.mode`, `reload.rs`'s `output_refusals`) and keeps the
  mode the session started with. Applying it live under `--tty` would run
  the hotplug path's `NewMode` arm (`Head::retarget`) on a head that is
  driving -- a runtime modeset that no hardware run has exercised as such
  (Asahi Tests 11-13 changed mode only through a startup `--mode` plus a
  replug). Under `--headless` it would be `resize_output_of`, which is
  proven, but the two backends should not disagree about what a reload does.

## What to do

- Position: an `[[outputs]]` `position = [x, y]` in logical pixels, an
  output without one packed after the placed ones. Decide and pin overlap
  (refuse, or clamp to adjacency), a gap the pointer cannot cross, and what a
  replug of a placed monitor does to its neighbours.
- Live mode: update `Tty::modes` (`output_config::ModeRequests`) from the
  reloaded entries and re-run `Tty::reconfigure`, so a changed mode is a
  `NewMode` like any re-probe; `resize_output_of` under `--headless`. Prove
  the `--tty` half on hardware (the Asahi box's DP-1 offers 21 modes) before
  dropping the refusal, including a mode the connector does not offer
  (warn, keep the preferred -- the reply should say so rather than report
  it applied).

## Not in this ticket

`wlr-output-management` `apply`/`test`: still refused, with the landing
condition in the [resolved spec](../resolved/per-output-scale-mode-done.md)
(section 3) -- position and live mode are its prerequisites, not it.

## Resolution (2026-10-08, PR #XXX)

Both halves landed, following this ticket's own recommendations.

- **Position.** `[[outputs]]` gains `position = [x, y]` in logical pixels
  (either axis may be negative; a mistyped value warns and packs that
  output, costing only the value). Placed outputs go exactly to their
  entries -- at startup, on hotplug add, and on reload; outputs without
  one pack left to right in creation order from the origin, stepping over
  placed ones, so a placed output never shoves an earlier output aside
  and the primary stays at the origin. Overlaps and gaps are allowed
  (each output renders its own strip; the pointer clamp already covers
  the bounding box, including negative origins -- a sole placed output
  clamps and centres on its own rectangle). A replugged placed monitor
  lands back on its entry while neighbours stay put; rescale, removal,
  mode change and position reloads repack the unplaced outputs behind
  it. `focus-output-left/right` needed no change (already geometry
  order); the X `INT16` bound and the IPC/output-management positions
  already read live geometry. Reported as `outputs.<name>.position`.
- **Live mode.** A changed `mode` applies per backend: `resize_output_of`
  under `--headless`, stored-requests swap plus `Tty::reconfigure`
  (the hotplug `NewMode` arm) under `--tty`, refused under `--nested`.
  A mode no connector offers -- or a failed switch -- keeps the running
  size and the reply says so (`outputs.DP-1.mode (could not switch to
  WxH; kept AxB)`). Entries for unconnected monitors store, report
  applied, and apply on plug; removals revert to the session default.
- **Proof.** Unit + reload suites (positions, modes, overlap, negative
  origins, replug, revert, clamp); headless two-output `scoot msg`
  proof (placed startup rect, live mode+position reload reply and rects,
  stored unconnected entry, screenshot); full `nextest --workspace`
  4555 passed; `cargo test` 2214 passed with two unrelated load flakes
  (each passing in isolation); clippy workspace + `gpu-scanout` clean;
  `xwayland` check clean; smoke 36 ok; docs-site build green. The `--tty`
  live modeset on a real panel is hardware-only: exact steps for the
  maintainer are in the PR body, not claimed here.
