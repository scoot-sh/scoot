---
title: "Skeleton: one bar layer surface per output, across hotplug"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M1"
resolved: "2026-09-29"
---

# Skeleton: one bar layer surface per output — RESOLVED

Resolved 2026-09-29. What landed:

- **`crates/scootbar`**: `scootbar daemon`, Linux only (a stub elsewhere),
  `#![forbid(unsafe_code)]`, the workspace release profile, and no new
  dependency: the Wayland crates, `rustix` and `scootbg-mem`'s sealed-memfd
  buffer are scootbg's
  ([the record's §7](dependencies-done.md#7-the-wayland-client-the-skeleton)).
  One `poll` over the Wayland fd, no timers, no frame callbacks.
- **A `top`-layer surface per output**, namespace `scootbar`, anchored to
  its edge and both sides, with margins and an exclusive zone of the bar's
  height set before the buffer-less first commit. The zone is the height,
  not height plus margin: the protocol says the compositor adds the
  anchored edge's margin, and the pinned Smithay's `arrange` (`035d447`,
  `src/desktop/wayland/layer.rs`) and sway both do, pinned by a test on
  each.
- **scootbg's per-output lifecycle, ported and trimmed** (settle round
  trip, staged `wl_output` properties, `closed` retried once then given
  up, ids never reused) as a pure model with a draw plan. The two copies,
  file by file, are listed in [extract-scootui](../extract-scootui.md) for
  the day it lands; scootbg was not touched.
- **Device pixels**: fractional scales through `wp_fractional_scale_v1`
  and a viewport (1067 × 28 logical at 1.5 is a 1601 × 42 buffer), the
  integer scale without them. **A pooled double buffer** per output, made
  on need, reused in place, never written while held, dropped once
  released at a stale size; **damage on change only**: an output is drawn
  when its frame (size, scale) changes and at no other time.
- **Flags, no config file**: `--edge top|bottom`, `--height` (1–1024,
  default 28), `--margin` (CSS shorthand, comma-separated, 0–1024 each),
  `--background '#rrggbb'`. Reference: [cli.md](../../cli.md); tests:
  [testing.md](../../testing.md).
- **CI**: a `scootbar` path filter in the classify job, a `scootbar` job
  (fmt, clippy, nextest and `cargo test`, no `libc` crate, `ldd` of the
  release binary) and a `scootbar-integration` job on headless scoot and
  sway.

**Deviations from the entry as filed**, each deliberate:

- **No `--font` yet.** The skeleton draws no text, and the entry's own
  refusal rule ("with no usable font the bar refuses to start") would
  refuse a bar that needs no font; the font mapping decision is
  [module-api-and-clock](module-api-and-clock-done.md)'s by M0's note. The
  flag, its well-known-directory fallback and the refusal land with the
  clock.
- **Runtime hotplug is tested on sway, not scoot.** scoot's headless
  backend cannot add or remove outputs at runtime (scootbg's `hotplug.rs`
  says the same); everything else in the Tests section runs on headless
  scoot, the window placement beside a margin included.

## Evidence

At `62181e6` (the code as merged, docs aside), in a Claude Code web container
(4 vCPU Xeon, Linux 6.18.44, the devenv shell's glibc 2.42), the same machine
as M0's baselines.

**Checks** (`CARGO_TARGET_DIR=/tmp/sb-skel-target`, `devenv shell --`):
`cargo fmt --check -p scootbar` clean; `cargo clippy -p scootbar
--all-targets -- -D warnings` clean; with `SCOOTBAR_REQUIRE_SCOOT=1
SCOOTBAR_REQUIRE_SWAY=1 SCOOTBAR_TEST_SWAY=<sway 1.12 from the pinned
nixpkgs>`, `cargo nextest run -p scootbar`: 71 passed, 0 skipped;
`cargo test -p scootbar`: 58 unit, 9 on scoot, 4 on sway, all passed.

**Benchmark**: M0's harness (`docs/scootbar/spikes/m0/bench/bar-bench.py`,
only its scratch log path changed) over the release binary (602,832 bytes,
stripped), `scoot --headless --outputs 1 --width 1920 --height 1080` (debug
build of the same tree, pixman) and headless sway 1.12 (pixman, 1920×1080),
each with two `foot` windows on two workspaces; 5 startups, then a fixed
30 s settle, a 300 s idle window and 240 workspace switches in 60 s:

```
=== scootbar 62181e681bffd57435224f388f23c9aa969a94a4 (tree clean: 0 changed crate files); binary 602832 bytes
=== scoot start 2026-09-29T05:23:32Z
startup_ms=[7.1, 8.5, 15.2, 7.2, 7.2]
procs=1 names=['scootbar'] threads=1
idle window_s=300.0 vol=0 nonvol=0 wakeups_per_min=0.00 cpu_ms=0.00 ticks=0
mem rss_kb=2756 pss_kb=1069 anon_kb=168 hwm_kb=2756
switch n=240 window_s=60.0 vol=0 cpu_ms=0.00 ticks=0 rss_kb=2756 hwm_kb=2756
=== sway start 2026-09-29T05:30:09Z
startup_ms=[6.9, 7.0, 8.7, 9.5, 7.0]
procs=1 names=['scootbar'] threads=1
idle window_s=300.0 vol=0 nonvol=0 wakeups_per_min=0.00 cpu_ms=0.00 ticks=0
mem rss_kb=2768 pss_kb=1081 anon_kb=176 hwm_kb=2768
switch n=240 window_s=60.0 vol=0 cpu_ms=0.00 ticks=0 rss_kb=2768 hwm_kb=2768
```

Against M0's baselines ([the table](../../README.md#baselines)), yambar, the
leanest, idles at 13.9 MiB RSS, 1.8 MiB heap and 2.6–4 wakeups a minute, with
a 21–29 ms first frame. The skeleton does less than any of them (no text), so
this is a floor, not the M1 comparison.

**Not measured or not verified:** real hardware (no `--tty` or dev VM run:
the bar is a plain Wayland client, and every run here is headless and
pixman); a compositor that `closed`s a bar on a live output (the retry path
is unit-tested on the model only); the Nix closure (no package yet,
[nix-package](nix-package-done.md)); an earlier run at `d7384ad` gave the same
scoot numbers (0 wakeups, 2,764 KiB RSS) but its sway phase was void (sway
could not bind its IPC socket under the long scratch path), so the whole
run was repeated at `62181e6`.


**Review follow-up** (PR #323): on sway 1.12, side margins wider than the
output were configured as a width of 4294966528 (-768 as a `uint`) and the
bar refused to draw; any configured side past `i32::MAX` is now taken as 0
and resolved to the 1-pixel bar, pinned by a unit test and a sway test that
fails without the fix. The same review found the window-placement test
leaking a `foot` per run; the harness now kills the compositor's children
first, and the test closes its window.

The entry as filed:

Filed 2026-09-29. Serves **daily-drive**.

The crate, a `scootbar daemon` that connects, tracks every output as it comes
and goes (as scootbg does, `crates/scootbg/src/outputs.rs`), and gives each
one a `top` layer surface anchored to a chosen edge with an exclusive zone
and namespace `scootbar`. It draws a solid bar and nothing else.

## What to do

- Crate `crates/scootbar`, Linux only (a stub elsewhere, like scootbg), the
  same release profile and `unsafe` discipline; shm buffers via the existing
  `scootbg-mem` pieces if they fit, otherwise the smallest local equivalent.
- Draw at real device pixels including fractional scales (`wp_viewporter` +
  `wp_fractional_scale`), a pooled double buffer, damage on change only.
- Bar height and edge from fixed defaults, overridable by flags (no config file yet); the exclusive zone must equal the
  bar so windows never jump when the first buffer lands (scoot applies the zone
  from the buffer-less initial commit, `layer-surface-bufferless-exclusive-zone-done.md`).
- Reuse scootbg's per-output lifecycle rather than reinventing it; note what
  would move into a shared crate later ([extract-scootui](../extract-scootui.md)),
  but do not refactor scootbg here.

## Margins from the first milestone

The skeleton sets the layer surface's margin from a flag (`--margin`, one to four
values, default 0) with `set_margin`, and the exclusive zone accounts for it as
[appearance](appearance-done.md) describes. That keeps every later look (floating bar,
matching scoot's gaps) a config change, not a rewrite. Test with a margin on
headless scoot: the zone, the window placement beside it, and the surface size
equal to the bar's (no transparent border).

## Options are flags until the config file exists

The first milestone has no config file. Edge, height, colors and the font path
are command-line flags with fixed defaults; the config file
([config-cli-and-reload](../config-cli-and-reload.md)) arrives in a later
milestone and the flags stay as overrides. With no usable font the bar refuses
to start and says how to give it one (see [nix-package](nix-package-done.md)).

## Tests

Headless scoot: the surface exists per output, the zone shifts a window, an
output added and removed at runtime adds and removes a bar, and a screenshot
over IPC has the bar color where expected (scootbg's integration pattern).
Edge cases: zero outputs, an output removed mid-frame, a scale change.

## Done when

`scootbar daemon` on headless scoot shows a solid bar on each output, and
idle wakeups are zero.
