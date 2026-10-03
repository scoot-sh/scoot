---
title: "Extract `scootui`: the shared drawing and Wayland scaffolding"
status: "open"
area: "scootbar"
priority: "low"
blocked: "waits for a second consumer (the launcher or the notification daemon) to start; unblock by hand when one does"
milestone: "M7"
---

# Extract `scootui`

Filed 2026-09-29. Serves **daily-drive** (one look across the shell) and the
lightness bar (no duplicated code in three binaries).

Write the drawing and scaffolding as modules inside `scootbar` first. Extract
when a second consumer needs them, so the seam comes from real use rather
than guesswork.

## Likely shape

- **`scootui`**: pure drawing with no Wayland: a canvas over `&mut [u32]`,
  rects and rounded rects, the glyph cache and text layout, color tokens, and
  the popup widgets. Snapshot-testable and fuzzable.
- **Wayland scaffolding**: per-output layer-surface lifecycle across hotplug,
  the shm pool, fractional scale. scootbg already has most of it
  (`crates/scootbg/src/outputs.rs`); decide whether it joins the extraction.
- **Line-framed JSON socket framing**, once a third daemon needs it (scootbg,
  scootbar, scootnotify).

## What is duplicated today

The [skeleton](resolved/skeleton-layer-surface-done.md) reused scootbg's
per-output lifecycle by porting it, not by sharing it (scootbg was not
refactored), so these are two copies to fold into one when this lands:

| scootbar | scootbg | What |
|---|---|---|
| `src/outputs.rs` | `src/outputs.rs` | The pure per-output model: staged `wl_output` properties, the settle round trip, the surface states (`closed` retried once, then given up), ids never reused. scootbar's is trimmed (no wallpaper choices, no waiting replies, no `xdg_output`) and adds the draw plan. |
| `src/density.rs` | `src/density.rs` | Scale arithmetic: `Preferred`, `Scale`, `scaled_length`. scootbar's leaves out `falls_short` (it rests on a surface covering the whole output). |
| `src/daemon/surfaces.rs` | `src/daemon/surfaces.rs` | The Wayland glue for the model: object lifetimes, stale-event filtering by id and live object, `release` on removal. |
| `src/daemon/canvas.rs` | `src/daemon/canvas.rs`, `src/share.rs` | The shm pool: `scootbg-mem` buffers, the fd closed once pooled, never written while held. scootbar's is a per-output double buffer; scootbg's shares pixels across outputs. |
| `src/daemon/mod.rs` | `src/daemon/mod.rs` | The `poll` loop's Wayland half (prepare-read, the exit on a hang-up with nothing left to read, flush with `POLLOUT`). |
| `src/print.rs`, `src/color.rs` | the same | Panic-free printing; `#rrggbb` parsing. |
| `tests/common/` | `tests/common/` | The headless scoot and sway harness and the screenshot reader. |

The [clock](resolved/module-api-and-clock-done.md) added scootbar's pure
drawing, all of it `scootui`'s likely core, with no copy in scootbg:
`src/paint.rs` (the canvas: span fills, coverage blending), `src/text.rs`
(the bounded glyph cache and one line of text), `src/layout.rs` (the
three sections), `src/theme.rs` (the color tokens and state classes) and
`src/render.rs` (views to pixels with per-module repaint and damage), plus
the seven-segment test font `src/testfont.rs` that pixel tests read back.

[Popups](resolved/popups-done.md) added the widget half, also pure and
Wayland-free: `src/popup/` (the declarative `Content` a module fills, `Layout`,
`Interaction`, `paint`), whose only dependencies are `paint`, `text` and `theme`,
so it moves with them. It is **not** extracted yet, for the reason this entry
already gives: the popups are the bar's own second consumer of its drawing, not
a second *binary*, and the seam is cut by the launcher or the notification
daemon wanting it. Its Wayland half (`src/daemon/popup/`: the `xdg_popup`, its
buffers, the pointer and keyboard routing) is the bar's and stays there.

`scootbg-mem` is already shared (scootbar depends on it for its buffers,
and since the clock for mapping a font that is a root-owned, unwritable
file on a read-only mount).
Its memfds are named `scootbg-wallpaper`, which is how scootbar's show in
`/proc/PID/maps`; a name parameter belongs with the extraction.

## Guard rails

scootbg has a release gate. Touching it means re-running its benchmark and
showing no regression beyond the margin, or leaving it on its own copy and
saying so. Feature-gate the parts a consumer does not use so the smallest
binary does not grow.

## Done when

Two binaries build on it, both benchmarks are unchanged within the margin, and
no consumer pays for a part it does not use.
