---
title: "Extract `scootui`: the shared drawing and Wayland scaffolding (epic)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-11-04"
---

# Extract `scootui`

Filed 2026-09-29. Serves **daily-drive** (one look across the shell) and the
lightness bar (no duplicated code in three binaries).

## The plan (2026-10-12)

The second consumers arrived: scootnotify, scootlaunch and scootgreet. The seam is cut in
[dev/design/shell-apps.md](../../../../dev/design/shell-apps.md#the-shared-crates-scootui-and-scoot-dbus) into three
crates:

- `scootui`: drawing, widgets, icons, Wayland scaffolding, keyboard and the control socket,
  each behind a feature;
- `scoot-dbus`: the D-Bus client;
- `scoot-rt`: the leaf both depend on (`print`, `runtime_dir`).

scootbg stays on its own copy for now. **Nothing is extracted until one of the three gates says
go.**

Children, in order:

1. [scootui-keyboard-spike](scootui-keyboard-spike-done.md) and
   [scootui-text-shaping-spike](scootui-text-shaping-spike-done.md) (NO-GO, measured 2026-10-18), now.
2. [scootui-extract-draw](scootui-extract-draw-done.md), then the moves in parallel:
   [scoot-dbus-extract](scoot-dbus-extract-done.md), [scootui-extract-wayland](scootui-extract-wayland-done.md)
   and [scootui-extract-control](scootui-extract-control-done.md). These are mechanical, with nothing
   new.
3. New code: [scootui-surfaces-on-demand](scootui-surfaces-on-demand-done.md),
   [scootui-keyboard](scootui-keyboard-done.md), then [scootui-entry-and-list](scootui-entry-and-list-done.md).

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

The [skeleton](skeleton-layer-surface-done.md) reused scootbg's
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

The [clock](module-api-and-clock-done.md) added scootbar's pure
drawing, all of it `scootui`'s likely core, with no copy in scootbg:
`src/paint.rs` (the canvas: span fills, coverage blending), `src/text.rs`
(the bounded glyph cache and one line of text), `src/layout.rs` (the
three sections), `src/theme.rs` (the color tokens and state classes) and
`src/render.rs` (views to pixels with per-module repaint and damage), plus
the seven-segment test font `src/testfont.rs` that pixel tests read back.

[Popups](popups-done.md) added the widget half, also pure and
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

## Resolution

Resolved 2026-11-04 after auditing "Done when" against the tree at `73569ff79`
(v0.7.3). The extraction landed in the nine children above; this records the
check of the three clauses.

**Two binaries build on it.** Three do: `scootbar`, `scootnotify` and
`scootlaunch` depend on `crates/scootui` (`cargo metadata` over the workspace
lists exactly those three, plus each one's `dev` dependency on `test-support`).
`scootgreet` does not exist yet and is not part of this epic's bar.

**No consumer pays for a part it does not use.** Each depends on `scootui` with
`default-features = false` and names what it uses:

| Consumer | `scootui` features | Left out |
|---|---|---|
| `scootbar` | `icons`, `wayland`, `control` (and `widgets`, `png`, `tray` through its own `popup`, `icon-image`, `tray` features) | `surfaces`, `keyboard`, `entry` |
| `scootnotify` | `surfaces`, `icons`, `png`, `control` | `wayland`, `keyboard`, `entry`, `widgets`, `tray` |
| `scootlaunch` | `entry`, `surfaces`, `keyboard`, `icons`, `png` | `control`, `wayland` (implied by `keyboard`), `widgets`, `tray` |

`cargo tree --target x86_64-unknown-linux-gnu -p <app> -e normal --prefix none`
shows `xkbcommon` in `scootlaunch` only, and no `xkbcommon` in `scootbar` or
`scootnotify`. `scootui::wayland` was enabled in `scootnotify` without being
called: that feature is dropped in #923 (same size, same libraries; the table lists scootnotify's features after it). Shipped builds are one package each, so Cargo never unifies one
app's `scootui` features into another: `nix/scootbar.nix`, `nix/scootnotify.nix`
and `nix/scootlaunch.nix` pass `-p <app>`, and `release.yml` builds
`cargo build --release --locked --offline -p "$bin"` per binary. (CI's
multi-package `cargo build -p scoot -p scootnotify -p scootbar` lines are
test setups, not shipped artifacts.) The children recorded the same property
as it landed: `scootbar`'s release binary byte-identical to `main`'s after
`surfaces-on-demand`, `keyboard` and `entry-and-list`, and no
`xkbcommon` edge in `cargo tree -p scootbar`.

**Both benchmarks unchanged within the margin.** Measured on the dev VM
(aarch64, 6 CPUs, `cgroup2`, scoot 0.7.3 headless as the only client), A-B-B-A for
`scootbar` and A-B-A for `scootbg`, A = `7bc849614` (the claim commit just before
`extract-draw`, 2026-10-18), B = `73569ff79` (v0.7.3), release builds in their own
target dirs, the harnesses from B. Raw results, logs and commands are in
[`dev/benches/scootui-extraction-vm/`](../../../../dev/benches/scootui-extraction-vm/README.md).
The history between A and B is confounded: it holds the epic's moves and also
unrelated work on `scootbar` and `scootbg` (look depth tokens #861, popup entrance
#863, bar opacity #850, floating-bar shadow #878, the allocation-failure audit
#894, #788 for `scootbg`). The harness's own noise rule (margin = the largest of 5%
of the value, the spread, and a floor) decides "same".

- `scootbar` (`scripts/scootbar-bench`, clock scope, 5 startups, 300 s idle, 240 workspace
  switches; `compare` verdicts):

  | Row | A, run 1 / 2 | B, run 1 / 2 |
  |---|---|---|
  | Bare executable, stripped (B) | 3,019,624 / 3,019,624 | 3,150,728 / 3,150,728 |
  | Size with non-glibc `ldd` closure (B) | 3,152,720 / 3,152,720 | 3,283,824 / 3,283,824 |
  | Startup to first frame (ms) | 28.8 / 38.8 | 30.1 / 40.4 |
  | Idle RSS / PSS (MiB) | 4.7 / 3.1 | 4.8 / 3.2 |
  | Idle wakeups per minute | 2 / 2 | 2 / 2 |
  | Idle CPU in the window (ms) | 4.7 / 3.8 | 4.0 / 3.1 |
  | CPU while switching workspaces (ms) | 0.9 / 0.2 | 1.3 / 0.2 |

  Verdicts: B run 1 against A run 1 flags one row (switching CPU, 0.9 to 1.3 ms); B run 1
  against A run 2 flags two (switching CPU, 1.3 against 0.2, and idle CPU, 4.0 against 3.8
  ms); B run 2 against either A run flags none. Everything else is `same`. The size grew
  131,104 B (+4.3%), inside the 5% margin the harness applies, and idle RSS and PSS +0.1
  MiB each. The flagged rows are this box's noise: switching CPU is 0.9 and 0.2 on the
  *same* binary A (run 1 and run 2), and 1.3 and 0.2 on B; no CPU row separates the trees
  from run-to-run variation.
  The size growth is not the extraction: every child measured its own step against its
  own base (the table above), and the steps sum to under 5 KB; the rest came with the
  feature commits listed above. The step from 3,019,520 B (keyboard) to 3,085,056 B
  (entry-and-list) is +65,536 B, one alignment step, and also not from the epic (both
  children recorded byte-identical binaries against their own `main`).
- `scootbg` (`scripts/scootbg-bench`, `--only startup,restore,idle --daemons scootbg
  --rounds 5`, 1x1080p and 2x4K, color and the 6000x4000 JPEG recorded in the
  harness): 0 regressions beyond the margin for B against A run 1, B against A run 2 and A run 2
  against A run 1. Size and Disk rows are identical (2,038,672 and 2,038,763 B; the
  binary is 1,905,576 B in both). Restore CPU for the JPEG: A 639 / 585 ms, B 601 ms;
  startup to a color on screen: A 34.6 / 37.2 ms, B 33.3 ms; idle wakeups and CPU
  0 in every cell. B is *better* on idle memory (RSS 5.4 vs 3.5 MiB and PSS 4.1 vs
  2.1 MiB with a color; RSS 37.4 vs 35.9 MiB with a 2x4K image), which `scootbg`'s own
  commits in the window account for (not `scootui`, which it does not use; the cause
  was not chased). The `2026-10-08-restore-idle` runs were on the Asahi M2 and
  cannot be compared with a VM, so the A-B pair on one machine stands in for them.
- `scootbg` is not on `scootui` (`cargo tree -p scootbg -e all` has no `scootui`), so
  the guard rail's "leave it on its own copy and say so" branch applies. `scootbg-mem`
  is shared and did change (`ShmBuffer::with_name`; `ShmBuffer::new` still calls it with
  `scootbg-wallpaper`; the test-only `refuse_from`); the A-B run above is what
  shows it cost `scootbg` nothing.

The popups paragraph and the duplication table above describe the state when
the epic was written (2026-09-29 to 2026-10-12); `crates/scootui` now holds
what they list.

## Theming contract (added 2026-10-12)

The color tokens this crate draws with come from the look layer, not a
palette of its own: see the [live looks design](../../../../dev/research/live-looks-design.md),
section 9, and the epic [`live-looks`](../../../backlog/core/live-looks.md).
