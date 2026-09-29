---
title: "Module API, layout, theme tokens and the clock"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M1"
resolved: "2026-09-29"
---

# Module API, layout, theme tokens and the clock — RESOLVED

Resolved 2026-09-29. What landed:

- **The module API** (`crates/scootbar/src/modules/mod.rs`): `init` returns
  `Available(Box<dyn Module>)` or `Unavailable(why)` (said once, takes no
  space and no fd); `sources` adds fds to the loop's poll set, asked every
  turn so a module's set may change; `on_ready(source, events)` returns
  `Changed` or `Unchanged`; `view(output, &mut View)` fills text, an
  optional icon, a state class and a tooltip, bounded to 256 bytes each
  and reused, never pixels. `init` must not block: a slow module returns
  `Available` with an empty view (no space) and joins from `on_ready`.
  One registry line and one Cargo feature per module (`clock`, the
  default); `--no-default-features` builds and is clippy-checked in CI.
  `on_input` is left for pointer-and-interactions as a defaulted method;
  `view` takes the output for the per-output workspaces module; the
  render keeps each module's span for hit tests and the agent interface's
  `layout`.
- **Layout, theme, text**: `left`, `center`, `right` (`--left`,
  `--center`, `--right`), `--padding`, `--spacing`; non-overlapping spans,
  clipped deliberately. Tokens `bg`, `fg`, `accent`, `dim`, `urgent`, with
  the classes `normal`, `warn`, `urgent`, `muted` mapped onto them
  (`--background`, `--foreground`). Text through `ab_glyph`, grayscale, no
  shaping, a glyph cache filled lazily and bounded (512 glyphs, 4 MiB,
  dropped and refilled past either), and no glyph rasterized whose bounds
  pass 4096 pixels a side or 4 megapixels (a hostile font's). The layout is recomputed only when a
  measured width, the scale or the bar's width changes; a change repaints
  and damages only its module's span (seen on the protocol trace:
  `damage_buffer(920, 0, 79, 28)` per tick on a 1920-wide bar).
- **The clock**: absolute `CLOCK_REALTIME` timerfd with cancel-on-set on
  the next local minute (second, if the format shows seconds), re-checked
  after arming (a boundary or step between reading the clock and arming it
  re-renders; the spike's loop missed the boundary case). M0's TZif reader
  ported with its M1 notes: capped while reading (at most 64 KiB + 1 from a
  regular file, `O_NONBLOCK`), instant clamped before any arithmetic,
  nothing indexed; `TZ` honoured as glibc does; the zone file `statx`ed on
  each wake. Format: a `strftime` subset, **default `%-I:%M %P`
  (`3:07 pm`)**, `%H:%M` one flag away, no locale.
- **Fonts**: `--font PATH`, a fixed list of well-known files without it,
  and a refusal to start (exit 1) naming how to give one when a module is
  placed and none is usable; no font needed with no module placed.
  **The mapping decision** (the Done when): mapped only when the file is
  on a read-only mount, owned by root and writable by no one (the Nix
  store), read into the heap everywhere else; the reasons and numbers are
  in [the dependency record's §8](dependencies-done.md#8-the-font-file-decided-the-clock).
- **Tests**: 137 unit (pure drawing read back from pixels through a
  seven-segment font built in code, the module harness with a trivial
  second module, the TZif reader against checked-in `zdump` fixtures of
  twelve zones fat and slim, a property test of 20,000 format strings);
  `tests/clock.rs` on headless scoot and sway (the time read off
  screenshots, per-module damage, idle between minutes, a real `foot`
  window closed and checked gone, font refusals, a hotplugged output);
  `cargo fuzz` targets `format` and `tzif` (`crates/scootbar/fuzz`).
  Reference: [cli.md](../../cli.md); tests: [testing.md](../../testing.md).

**Review of #324**, fixed in `de27775`:

- **The first mapping rule was unsafe.** It mapped any font on a
  read-only mount; review reproduced a `SIGBUS` (exit 135) with a font
  bind-mounted read-only and truncated through its read-write path, and
  silent corruption from a smaller `cp` over it. The rule now also needs
  the file to be root's with no write bit, which store files are; the
  predicate is a pure function tested for each exclusion, and the repro is
  recorded below against both commits.
- **The uncached glyph path could allocate without bound**: `ab_glyph`
  allocates an `f32` per pixel of a glyph's bounds, and the bounds come
  from the font. Now capped (4096 a side, 4 megapixels), with a hostile
  test font; the cache takes glyphs up to 1 MiB so `--font-size 256` at
  scale 2 is rasterized once, not per draw.
- One scale for measuring, painting and damage; `--help` matches the
  build's modules; `TZ=UTC` (and friends) with no zone data, and an absent
  `/etc/localtime`, are UTC without a warning.

**Deviations from the entry and the brief**, each deliberate:

- **Idle is two wakeups a minute, not one.** The target is two, ratified by the
  maintainer ([decisions](../lightest.md#decisions)); this is what was
  measured (review of #324): one timer wake a minute, and each frame
  brings one `wl_buffer.release`. The tick, then 0.9 to 1.2 ms
  later on scoot (0.18 to 0.25 ms on sway) the compositor's
  `wl_buffer.release` for the buffer that tick's commit replaced (trace
  below). M0's one-a-minute was measured with no Wayland connection. The
  only protocol-legal way to avoid the release found is a fresh buffer per
  frame with the old one destroyed before the commit, which costs a
  memfd, a mapping and its page faults per frame on both sides, more real
  work than the wakeup it saves, and gives up the pooled buffers; not done.
  It is still the fewest wakeups of any bar measured (yambar 2.6 to 4).
- **The layout is recomputed on width changes only** and a width change
  repaints and damages the whole bar (a span moved); rare for a clock with
  tabular digits.
- **Only `bg` and `fg` are flags**: the other tokens get keys with the
  config file.

## Evidence

In a Claude Code web container (4 vCPU, Linux 6.18.44, devenv shell's
glibc 2.42), the same machine as M0 and the skeleton. Checks were rerun on
the final commit (the PR says which); the benchmark below is at `44fc656`,
before two changes that touch no measured path (the layout skipped when no
width changed; the opt-in step test).

**Benchmark**: M0's harness (`docs/scootbar/spikes/m0/bench/bar-bench.py`,
only its scratch log path changed) over the release binary (848,624 bytes,
sha256 `13db2359b160775a...`), `scoot --headless --outputs 1 --width 1920
--height 1080` (debug build of the same tree) and headless sway 1.12
(pixman, 1920×1080), two `foot` windows on two workspaces each; `scootbar
daemon --font <DejaVuSans.ttf from nixpkgs> --clock-format '%a %d %b
%H:%M'`, `TZ` unset (`/etc/localtime` is UTC); 5 startups, a fixed 30 s
settle, a 300 s idle window, 240 workspace switches in 60 s:

```
=== scootbar 44fc656d29a6e6c140bcc6906493cc0297d4d515 (tree: 0 changed files); binary 848624 bytes sha256 13db2359b160775a
=== scoot start 2026-09-29T07:15:16Z
startup_ms=[9.9, 10.1, 9.7, 13.3, 9.7]
procs=1 names=['scootbar'] threads=1
idle window_s=300.0 vol=10 nonvol=0 wakeups_per_min=2.00 cpu_ms=1.96 ticks=0
mem rss_kb=3944 pss_kb=2108 anon_kb=940 hwm_kb=3944
switch n=240 window_s=60.0 vol=2 cpu_ms=0.30 ticks=0 rss_kb=3944 hwm_kb=3944
=== sway start 2026-09-29T07:21:53Z
startup_ms=[8.7, 8.7, 9.9, 10.3, 7.3]
procs=1 names=['scootbar'] threads=1
idle window_s=300.0 vol=10 nonvol=0 wakeups_per_min=2.00 cpu_ms=1.46 ticks=0
mem rss_kb=3960 pss_kb=2126 anon_kb=940 hwm_kb=3960
switch n=240 window_s=60.0 vol=2 cpu_ms=0.24 ticks=0 rss_kb=3960 hwm_kb=3960
=== scoot, font mapped from a read-only mount (/tmp/sb-rofont/DejaVuSans.ttf, ro,...) start 2026-09-29T07:28:31Z
startup_ms=[9.4, 11.9, 7.4, 7.7, 9.6]
procs=1 names=['scootbar'] threads=1
idle window_s=300.0 vol=10 nonvol=0 wakeups_per_min=2.00 cpu_ms=1.46 ticks=0
mem rss_kb=3388 pss_kb=1552 anon_kb=196 hwm_kb=3388
=== done 2026-09-29T07:34:06Z
```

The mapped run above used a root `0644` copy on a read-only bind mount,
which the review fix now reads rather than maps. The font paths were
measured again at `de27775` with a root `0444` copy (mapped: 3,432 KiB
RSS, 188 KiB heap) and the `0644` one (read: 3,924 KiB RSS, 932 KiB heap),
with the review's `SIGBUS` repro before and after the fix, in
[the dependency record's §8](dependencies-done.md#8-the-font-file-decided-the-clock).

Against the skeleton (2,756 KiB RSS, 168 KiB heap, 0 wakeups, 7.2 ms first
frame): +1.2 MiB RSS read (+0.6 MiB mapped), of which 744 KiB is the
font's bytes; +2 wakeups a minute; +2.7 ms to the first frame (the font
read and the first glyphs). Against yambar on scoot (13.9 MiB RSS, 1.8 MiB
heap, 4.0 wakeups, 28.5 ms): below on every row but the bare binary size
(849 KB against 407 KB, which links libwayland, pixman and fcft besides).

**Where the second wakeup comes from** (`WAYLAND_DEBUG=1`, `%H:%M:%S`,
same scoot):

```
[ 907017.338][rs] -> wl_surface@9.attach(wl_buffer@14, 0, 0)
[ 907017.470][rs] -> wl_surface@9.damage_buffer(920, 0, 79, 28)
[ 907017.489][rs] -> wl_surface@9.commit()
[ 907018.629][rs] <- wl_buffer@16.release, ()
[ 908017.395][rs] -> wl_surface@9.attach(wl_buffer@16, 0, 0)
[ 908017.436][rs] -> wl_surface@9.damage_buffer(920, 0, 79, 28)
[ 908017.457][rs] -> wl_surface@9.commit()
[ 908018.370][rs] <- wl_buffer@14.release, ()
```

and on sway `commit` at 912017.321, `release` at 912017.502.

**Binary size**, symbol sizes summed by crate (`nm -S` of unstripped
builds, the skeleton at `84452ee`, identical scootbar and scootbg-mem code
to `main`'s): total 455,309 → 661,102; `ttf_parser` +88,076, `ab_glyph`
+8,490, `ab_glyph_rasterizer` +5,323, `owned_ttf_parser` +1,118 (103 KB,
M0 said 115); scootbar's own +52,362; `core` +20,426, `alloc` +2,855,
`std` +5,769, `rustix` +3,569, other +19,504.

**Fuzzing**, at `44fc656`'s parsers (unchanged since), `cargo fuzz run -s
none`, 300 s each: `format` 11,738,471 runs, `tzif` 113,636,649 runs, no
crash, timeout or leak.

**Seen**: DejaVu Sans on headless scoot, the default format centered
(`7:34 am` at 07:34 UTC) and `--edge bottom --right clock --clock-format
'%a %d %b %H:%M:%S %Z'` (`Tue 29 Sep 07:34:56 UTC`), looked at by eye.

**Not measured or not verified**:

- **Clock steps and suspend on the real timer.** The opt-in test that sets
  the system clock (`clock_steps_and_summer_time_show_on_time`) was written
  but not run: setting the clock was refused in this environment. What
  covers the path: unit tests of the arithmetic across steps and DST
  boundaries, and M0's measurements of the same timerfd use (§2c of the
  dependency record). No suspend, no dev VM (neither VM port answered).
- Real hardware of any kind; a third compositor; the Nix closure (no
  package yet); a real DST change in real time.
- Every well-known font path: only Debian/Ubuntu's exists on this machine;
  the others are the distributions' documented package paths, unchecked.

The entry as filed:

Filed 2026-09-29. Serves **daily-drive**.

The contract every module is written against, proven by the first one.

## The trait

A module (one file in `modules/`, one registry line, one Cargo feature):

- `init` probes and returns `Available` or `Unavailable`; an unavailable
  module registers no fds and takes no space, so absent hardware costs nothing.
- `sources` names the fds and timers it wants polled.
- `on_ready(source)` handles one and returns whether the view changed.
- `view(output, &mut View)` fills a small declarative `View`: icon, text,
  state class (`normal`, `warn`, `urgent`, `muted`), optional tooltip. A module
  never touches pixels.
- `on_input` is added in [pointer-and-interactions](../pointer-and-interactions.md).

Built once at startup as trait objects (allocation at load or reload only),
never per frame. Dispatch cost is a handful of virtual calls per redraw.

## Layout, theme and text

`left`, `center`, `right` lists of module ids; per-module padding and
spacing; semantic color tokens (bg, fg, accent, dim, urgent, plus the state
classes) so a theme source maps onto them. Text through the rasterizer the
[spikes](dependencies-done.md) chose (`ab_glyph`), glyph cache lazily filled and bounded
(see [robustness-and-limits](../robustness-and-limits.md)), grayscale AA, no shaping. Recompute layout
only when a module's measured width changes; damage only the changed
module's rect.

## The clock

Absolute realtime timerfd on the next minute (second, if configured) with
cancel-on-clock-set, so suspend, NTP steps and DST are handled without
polling; timezone from `/etc/localtime`. Format string from a flag (a config key
later), with a small documented set of specifiers.

M0 measured this design (one wakeup per minute, 10 in 600 s) and chose the
parts; [the record](dependencies-done.md#what-this-changes-in-the-plan)
lists what to carry over: re-check the clock after arming, port the spike's
TZif reader with its `zdump` check as a fixture test, honour `TZ`, `statx`
the zone file on each wake, and prove suspend/resume on hardware (M0 could
only read the kernel for it).

## The font flag, from the skeleton

The [skeleton](skeleton-layer-surface-done.md) draws no text, so it
left the font to this entry: `--font PATH` on `scootbar daemon`, the short
fixed list of well-known directories when it is not given, and a refusal to
start, naming how to give it one, when no usable font is found
([nix-package](nix-package-done.md)). Document it in `docs/scootbar/cli.md`
beside the skeleton's flags.

## First frame first

Paint the bar and the clock before anything else is initialized. A module whose
`init` is slow (a bus connection, a scan) must never block the loop or delay the
first frame; it joins when ready. Slow startup is a recurring bar complaint
(Waybar #1093), and a bar that shows up late is not the lightest one.

## Tests

Pure-drawing snapshot tests (no Wayland), a module test harness that feeds
fake events and asserts on the `View`, a fuzz target over the format string,
and a headless-scoot pixel test showing the time. Measure: idle wakeups are
exactly one per minute.

## Done when

The clock renders on every output and adding a second trivial module is one
file, one line and a test. The font-mapping decision is made here, not left to
`robustness-and-limits`: a `cp` over the font file under the running bar is a
`SIGBUS`, so either read files outside `/nix/store` into the heap (about
+750 KB) or document the hazard.
