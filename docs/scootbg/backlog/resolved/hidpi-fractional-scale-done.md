---
title: "Drawing at real device pixels on scaled outputs"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Drawing at real device pixels on scaled outputs — RESOLVED

Resolved 2026-09-27. What landed, where it departs from the plan (the
ticket's snap to the output's mode was measured and dropped), what was
verified where, and the measurements are in [Resolution](#resolution) at
the end; the original ticket follows unchanged but for its links, which
now resolve from `resolved/`.

- With `wp_fractional_scale_v1` and `wp_viewporter`: allocate the buffer
  at the logical size times the scale, rounded halfway away from zero as
  the protocol specifies, set the viewport destination to the logical
  size, and redraw on `preferred_scale` changes. For a surface covering
  the whole output, that can land a pixel off the output's real mode
  (2560 px at 1.5 is 1707 logical, which rounds back to 2561): prefer the
  output's current mode size there, and test the rounding cases.
- Without fractional scale: `wl_surface.set_buffer_scale` with the integer
  `preferred_buffer_scale` (surface v6) or the output's scale.
- Today `wl_surface.preferred_buffer_scale`/`_transform` are ignored and
  `query`'s `logical` is an integer-scale estimate until the surface is
  configured ([ticket 3](outputs-and-layer-surfaces-done.md#for-the-next-tickets)).
- A scale change rescales from the source (re-decoding if it was dropped),
  never from the previous scaled buffer.

Check on scoot with a fractional `scale` in its config: the screenshot must
be pixel-sharp, not a compositor-upscaled blur.

**From [ticket 6](images-decode-and-fit-done.md#for-the-next-tickets):**
an image is drawn at the configured surface size times `wl_output`'s
integer scale, with `set_buffer_scale`, like a full-size color. At a
fractional scale that is larger than the output (1.5 on 1600×1000: a
2134×1334 buffer for a 1067×667 surface) and the compositor scales it
down, so it is sharp but not device-exact, and costs ~78% more pixels to
draw and hold than the output has. A new scale re-renders from the file
only when the buffer size changes (scale 2 with the mode doubled keeps
it, and attaches it again: Smithay-based compositors read a new
`set_buffer_scale` only with an attach, see
[the compositor item](../../../backlog/core/buffer-scale-needs-a-new-buffer.md)).
The size is decided in one place, `daemon::change::image_dims`, and the
buffer is always the worker's full-size render, so drawing at a
`wp_fractional_scale_v1` size is a change there plus a viewport
destination.

## Resolution

### What landed

Code at `c7c3132`; the docs came after it, with two comment-only fixes
in `canvas.rs` and `wayland.rs` (the release binary is byte-identical,
sha256 `ae084758…`, so every measurement here holds for both).

- **The size, decided once** (`src/density.rs`, pure, 12 unit tests in
  `density/tests.rs`). `Scale` is `Integer(n)` or `Fractional(v120)`, and
  `Scale::buffer(size)` is the one place a full-size buffer's size is
  worked out: a fraction is the logical size times `v120 / 120` per side,
  rounded halfway away from zero as `wp_fractional_scale_v1` says, at
  least 1, at buffer scale 1 under a viewport; an integer multiplies and is
  the buffer scale. `Output::full_buffer` (what `daemon::change::image_dims`
  now returns, so what a render is asked for, kept and offered at) and
  `Drawn::buffer` (what the draw attaches) both read it; a unit test pins
  that they agree on every path and scale
  ([why that matters](#found-along-the-way)).
- **Which scale** (`Preferred::scale`, `Output::scale`):
  `wp_fractional_scale_v1`'s `preferred_scale`, else the larger of
  `wl_surface.preferred_buffer_scale` (v6) and `wl_output.scale`; a
  surface scale that would draw the buffer *smaller* than the output
  (below `wl_output`'s, or short of the mode) is taken as stale and the
  integer used ([departures](#departures-from-the-plan-and-why)).
  The model (`outputs.rs`) keeps them per output, from the live surface's
  events only, across a surface re-created on the same output. A fraction
  of 0 or an integer below 1 is ignored.
- **The glue** (`daemon/surfaces.rs`): each surface gets a
  `wp_fractional_scale_v1` before its first commit, only where the
  compositor has a viewporter too and the chosen path uses one
  (`Globals::fractional_scale`), destroyed before its surface.
  `wl_surface`'s events are now read (user data `OutputId`); a changed
  scale redraws one round trip later, like a `done`. A `configure` is
  acked at once and drawn one round trip later too
  (`RoundTrip::Configured`, skipped if a newer `configure` came).
  `preferred_buffer_transform` is read and deliberately not acted on
  ([below](#departures-from-the-plan-and-why)).
- **The draw** (`daemon/canvas.rs`): from `Drawn::buffer`; a viewport
  sizes any buffer that is not the surface's size at its own buffer scale
  (a 1×1 color, a fractional buffer), and once a surface has one its
  destination stays the surface size. Colors on the single-pixel and 1×1
  paths are unchanged (scale 1, so a new scale does not redraw them); the
  full-size color path takes the fraction too where a viewporter exists.
  The re-attach for a buffer-scale change without a new buffer stays for
  the integer path; on the fractional path the buffer scale is always 1.
- **`query`** (additive to protocol 1): `surface.scale` (the scale drawn
  at: `1.5`, or an integer, whole ones printed as integers) and
  `surface.pixels` (a full-size buffer's size in device pixels), both
  `null` until configured; `logical` before a `configure` is worked out
  with the fraction when known (1067×667 at 1.5 on 1600×1000, not 800×500),
  which also sizes a surface configured 0×0.
- **A debug knob**, compiled out of release builds like
  `SCOOTBG_DEBUG_PATH`: `SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE` leaves the
  manager unbound, standing for a compositor without it; the forced
  `full-shm` path now also stands for one without a viewporter (no
  fractional-scale objects).
- **Tests**: `tests/scale.rs` (new, 7 tests, added to CI's integration
  job), `outputs/scale_tests.rs` (8), `density/tests.rs` (12), and the
  paint, protocol and daemon tests updated.
  `tests/image.rs::a_new_scale_redraws_at_the_real_pixel_size` now runs
  with the knob, so it still covers the integer path and the re-attach.
- **Filed for the compositor:**
  [fractional-scale-in-120ths.md](../../../backlog/core/fractional-scale-in-120ths.md)
  (below); [buffer-scale-needs-a-new-buffer.md](../../../backlog/core/buffer-scale-needs-a-new-buffer.md)
  updated (scootbg meets it only without the fraction now).

### Departures from the plan, and why

- **No snap to the output's mode.** The ticket said to prefer the mode
  where the rounding lands a pixel off it (2561 → 2560). Measured on scoot
  at 1.5 on 1600×1000, a one-pixel checkerboard the size of the output
  shown unscaled: with the protocol's 1601×1001, **0** of 1,600,000 pixels
  differ from the checker; with a temporary build snapping to 1600×1000,
  **1,599,903** do (the pattern turns grey: 1/253 then worse across the
  width). Both compositors draw a surface `round(logical × scale)` device
  pixels wide: Smithay's `WaylandSurfaceRenderElement::size` (pinned fork,
  `element/surface.rs:327`), 1067 × 1.5 → 1601, clipped at the edge; and
  wlroots, which truncates the logical size (`*width /= output->scale`,
  1066) and then rounds in `scale_length` (1599). A buffer that size lands
  one to one; the mode's is stretched or squeezed by a pixel. The
  protocol's rounding is exact for any scale that is a multiple of 1/120,
  and the unit tests check it against both compositors' rounding over 16
  modes, both orientations and every step from 60/120 to 480/120 (26,944
  cases).
- **A stale scale gives way to the larger.** Not in the ticket. wlroots
  sends a surface `preferred_scale` and `preferred_buffer_scale` only
  while it is on screen, so after `output … scale 1.5` on sway a surface
  showing nothing (never set, or cleared) or fully covered keeps its old
  scale (trace below). Two checks, both distrusting a scale only when it
  would make the buffer *smaller* than the output (stretched, a blur);
  a larger one is scaled down, sharp, and left alone:
  - *against `wl_output.scale`*: both compositors send it as the fraction
    rounded up, so a surface scale below it (1.0 against 2) is stale, or
    it is; the larger wins. A compositor that says 1 on `wl_output` and
    1.5 on the surface is followed;
  - *against the mode* (added after review, see
    [below](#review-of-pr-283)): a stale fraction that rounds up to the
    same integer (1.25 against 1.5, both 2) passes the first check, so a
    fraction whose buffer falls short of the output's mode by at least the
    scale plus half a pixel (`Scale::falls_short`: sway's 1066-wide
    surface at a stale 1.25 is 1333 for a 1600-pixel mode) gives way to
    the integer scale. The unit tests show a fraction the compositor
    renders at exactly (a multiple of 1/120, as sway always is) never
    trips it, over the same 26,944 cases. A compositor rendering *below*
    the 120th it sends does trip it, for good: scoot at `scale = 1.254`
    sends 150 and gets a 2552×1594 buffer at integer scale 2 for a
    1600×1000 output (review, measured), about 2.5× the fraction's
    pixels; neither is exact there. See
    [the compositor item](../../../backlog/core/fractional-scale-in-120ths.md).

  On sway either case draws 1066×666 at buffer scale 2 (2132×1332) first,
  and once on screen sway sends 180 and it is redrawn at 1599×999, exact:
  one extra render. So after a scale change made while nothing was shown,
  a screenshot straight after `set` shows the image drawn larger and
  scaled down, not yet device-exact.
- **A configure's draw waits one round trip.** Not in the ticket. Neither
  compositor sends the `configure` last in a scale change: sway sends it
  before `wl_output.done`, scoot before the surface's `preferred_scale`
  (traces below). The worker already drops a queued render whose size went
  stale before it starts, but only events in the same read reach it in
  time; the round trip makes the whole batch count however the reads split.
- **`preferred_buffer_transform` is not acted on.** The ticket said to
  stop ignoring it. Drawing pre-rotated only spares a compositor that
  composites in software a transformed copy of the buffer, and neither
  compositor here ever sends anything but `normal` for an shm buffer:
  wlroots sends the output's transform only with dmabuf feedback for
  scanout (`wlr_scene.c`, beside a TODO for software rendering), and
  scoot always sends `normal`. An untransformed buffer is correct
  everywhere, and code for the other seven transforms could not be checked
  against any compositor here. The event is read and the reason is in the
  code (`surfaces.rs`).
- **The size moved from `daemon::change::image_dims` into
  `density::Scale::buffer`.** `image_dims` still exists and still decides
  what renders are asked for and kept, but the draw needed the same answer
  from the pure model, so the arithmetic is in one pure function both call.
- **Scales are kept when a surface is re-created**, rather than reset:
  they describe the output, and both compositors send the new surface its
  own before its first `configure` anyway.

### Found along the way

- **The two size paths must agree, or a `set` hangs.** The first snap
  experiment snapped only in `Output::full_buffer`: the draw asked for
  1601×1001, the worker's results were kept only at 1600×1000, and the
  render was dropped and asked for again until the client gave up
  ("Resource temporarily unavailable" after 30 s). The shipped code has a
  single function, and `a_draw_asks_for_the_size_a_render_is_kept_for`
  pins it.
- **Event orders** (`WAYLAND_DEBUG=client` traces):
  - scoot, reload to 1.5, image on screen: `wl_output.scale(2)`, `done`,
    `configure(1067, 667)`, `preferred_scale(180)`,
    `preferred_buffer_scale(2)`;
  - sway, `output … scale 1.5`, surface on screen: `wl_output.scale(2)`,
    `preferred_scale(180)`, `preferred_buffer_scale(2)`,
    `wl_output.scale(2)`, `configure(1066, 666)`, `done`;
  - sway, the same with nothing on screen: `wl_output.scale(2)` twice,
    `configure(1066, 666)`, `done`, and no scale for the surface.
- **Smithay applies a new viewport destination without a new buffer**
  (`RendererSurfaceState::update_buffer` works the surface view out at
  every commit), unlike a new buffer scale. On scoot, 1 → 1.25 → 2 → 1 on a
  1600×1000 mode keeps one 1600×1000 buffer (decoded once) under
  destinations 1600×1000, 1280×800, 800×500, exact to the pixel at every
  step, with no re-attach.
- **scoot at a scale that is not a multiple of 1/120 cannot be drawn
  exactly by any client**: at 1.33 it renders at 1.33 and says 160/120,
  and the protocol-exact 1604×1003 buffer is squeezed into 1600 pixels:
  1,596,598 of 1,600,000 checker pixels off (0 at 1.25 and 1.5). Filed as
  [fractional-scale-in-120ths.md](../../../backlog/core/fractional-scale-in-120ths.md);
  no scoot code changed here.
- **sway at 1.5 on 1600×1000 leaves the output's last column and row
  uncovered** by any full-output layer surface (1066 logical × 1.5 =
  1599), whatever the client draws: wlroots' truncation, not scootbg's.

### Verified where

All on a Claude Code web container (x86_64, 4 CPUs), no dev VM; code at
`c7c3132`.

- **scoot `--headless`** (`tests/scale.rs`): at 1.5 the trace shows one
  `get_fractional_scale`, one buffer `1601, 1001, 6404`,
  `set_destination(1067, 667)` and no `set_buffer_scale`, `query` says
  `"scale":1.5,"pixels":{"width":1601,"height":1001}`, and the checker is
  exact over all 1,600,000 pixels; the round 1 → 1.25 → 2 → 1 → 1.5 is
  exact at every step, with one buffer until 1.5 and two after; a scale
  change 1.5 ms after an image `set` (the reply 1.85 s later: it happened
  mid-decode) gives only 1601×1001 buffers to the compositor, `ok`, and a
  sharp checker; with the knob, and on the forced `full-shm` path, an
  image is 2134×1334 at `set_buffer_scale(2)` (not exact, as expected) and
  a color is one flat color on screen. After each: one thread, no wakeups.
- **sway 1.12 headless** (wlroots 0.20.2, pixman,
  `/nix/store/5ddkfdxnq991rfzn2f6n5w1kd6dvaqp3-sway-1.12`): at 1.5 on
  1600×1000 the buffer is 1599×999 and the screencopy a sharp checker over
  it; at 1.25 1600×1000, sharp; an output plugged in at 1.5 (1920×1080) is
  drawn at 1920×1080 and the centred checker is sharp inside the fill; a
  scale gone stale while nothing was shown is drawn at 2132×1332 first,
  then 1599×999, sharp.
- **Mutation checks**: trusting the fraction unconditionally fails
  `a_stale_smaller_scale_gives_way_to_the_output` and
  `a_scale_gone_stale_while_unmapped_is_healed_on_sway`; removing the
  re-attach fails `a_new_scale_redraws_at_the_real_pixel_size`. Drawing at
  the `configure` again (no round trip) fails **nothing**: every batch
  arrived in one read here, so the worker's stale-size filter covered it
  (see [Not verified](#not-verified-and-why)).
- **Commands**, at `c7c3132`:
  `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1 SCOOTBG_TEST_SWAY=…/sway devenv shell -- soft-egl cargo nextest run -p scootbg -p scootbg-mem`
  (297 passed, 1 skipped: the `#[ignore]`d benchmark); `cargo test -p
  scootbg -p scootbg-mem` with the same variables (235 + 9 + 14 + 2 + 7 +
  3 + 6 + 17 + 4 passed, 1 ignored); `cargo clippy -p scootbg -p
  scootbg-mem --all-targets -- -D warnings` and `cargo fmt --check -p
  scootbg -p scootbg-mem` clean; `RUSTFLAGS="-D warnings" cargo build
  --release -p scootbg` clean, linking `libgcc_s`, `libm` and `libc`
  only, no `libc` crate; `NIX_GIT_SSL_CAINFO=… nix build .#scootbg
  --option sandbox true` builds (its one warning: the tree is dirty with
  this record's own move), 1,492,704 B, linking the same three.

### Review of PR #283

No blocking findings. Fixed, in a commit after `a1bf622`:

1. **A stale fraction that rounds up to the same integer slipped past
   "the larger wins"** (reproduced by the reviewer on sway 1.12). At 1.25,
   set a color, clear it (the new surface gets 150), `output … scale 1.5`
   while cleared: `query` said scale 1.25, pixels 1333×833, and `set` of a
   checker answered `ok` with that buffer stretched over 1599 device
   pixels; only after mapping did sway send 180. The same holds for a
   mapped wallpaper fully covered (wlroots suspends it). Fixed by the check
   against the mode (above). The reviewer's script, run again against the
   fix: after the scale change `query` says
   `"scale":2,"pixels":{"width":2132,"height":1332}`, the buffer at the
   reply is `2132, 1332`, and a second later `1599, 999`, with `query` at
   `"scale":1.5`. The new sway test
   `a_stale_fraction_that_rounds_alike_is_not_stretched_on_sway` fails on
   `a1bf622` twice over: `query` says 1.25, and with that assertion taken
   out, `a buffer stretched over the output: [(1333, 833)]`.
   *Considered and not done:* re-creating an unmapped surface when a
   `configure` changes its size (sway sends a new surface its scales at
   creation). It would make the first draw exact on sway, but it does not
   reach a covered, mapped wallpaper, it costs a surface and a round trip
   per change on every compositor, and on one whose fraction persistently
   disagrees with the mode it would have to be bounded against a loop.
   Drawing larger until the compositor says otherwise is right everywhere.
   *Why the mode check is sound:* the surface is anchored to all four
   edges with exclusive zone -1, so it is the whole output; only a buffer
   falling short counts, so scoot at 1.33 (1604 for 1600) keeps its
   fraction; and a surface a compositor configured smaller than the output
   degrades to the integer scale (larger, sharp), never to a stretch.
2. `density.rs` named `tests/image.rs` for the snap screenshots; it is
   `tests/scale.rs`.
3. README, `docs/scootbg/README.md` and `SurfaceEntry` said the integer
   scale was `preferred_buffer_scale` *else* `wl_output`'s; it is the
   larger of the two.
4. `surface.scale` was documented as the scale scootbg draws at; a color
   on the single-pixel or 1×1 path is drawn at 1. It is now documented as
   the scale an image, or a color on the full-size fallback, is drawn at.
5. A failed draw stayed failed after a new scale that could make it
   drawable (`failed` was cleared only by a request or a `configure`). A
   changed `preferred_scale`, `preferred_buffer_scale` or `wl_output.scale`
   now clears it (`Output::rescaled`); unit-tested.

Mutation checks on the fixes: skipping the mode check fails
`a_stale_fraction_that_rounds_alike_is_not_stretched_on_sway`,
`a_fraction_short_of_the_mode_gives_way_to_the_integer` and
`the_fraction_makes_the_estimate_and_sizes_an_unsized_surface`; not
clearing `failed` on a new scale fails `a_new_scale_retries_a_failed_draw`.

The covered variant of finding 1, reproduced by hand on sway 1.12 (not
in the test suite: it needs a client to cover the output with). With an
image on screen at 1.25, a fullscreen `foot` over it, then `output …
scale 1.5`, sway sends the covered surface no scale (the trace has only
the 150 from creation). `a1bf622` (release, sha256 `ae084758…`) then drew
`1333, 833` and `query` said `"scale":1.25`; the fix draws `2132, 1332`
with `"scale":2`. On uncovering, sway sends 180 and both redraw at
`1599, 999`.

### Measurements

The 6000×4000 JPEG of the earlier records, regenerated with the same
recipe (ImageMagick fractal plasma plus Gaussian noise, quality 92, 4:2:0;
7,925,275 B), `fill`, release builds. **Before** is `bc53961` (release
binary sha256 `bf5ecb9f…`), **after** `c7c3132` (`ae084758…`).

**Buffer bytes** at 1.5, from the daemon's memfd (`st_size`) after each
`set`, and `RssShmem`:

| Output | before | after | |
|---|---|---|---|
| 1600×1000 at 1.5 | 2134×1334: 11,387,024 B (11,124 kB) | 1601×1001: 6,410,404 B (6,264 kB) | −43.7% |
| 3840×2160 at 1.5 | 5120×2880: 58,982,400 B (57,600 kB) | 3840×2160: 33,177,600 B (32,400 kB) | −43.8% |

The old buffer had 77.6% (1600×1000) and 77.8% (4K) more pixels than the
new one, the ticket's ~78%.

**Draw time, in-process** (the `#[ignore]`d `image::bench::pipeline`,
`SCOOTBG_BENCH_SIZE` set to each buffer size, 5 runs; the pipeline code
is the same before and after, only the size it is asked for changed):

| Buffer | scale, ms | pack, ms | total, ms | peak, kB |
|---|---|---|---|---|
| 2134×1334 (before, 1600×1000) | 129.0, 116.9, 119.8, 120.0, 124.8 | 7.9, 7.6, 7.6, 7.5, 7.7 | 395.6, 364.5, 366.5, 362.3, 371.1 | 79,488, then 79,548 ×4 |
| 1601×1001 (after) | 99.7, 101.1, 96.1, 99.1, 97.0 | 4.3, 4.3, 4.2, 4.2, 4.3 | 344.0, 342.5, 339.1, 338.9, 342.7 | 75,984, then 76,140 ×4 |
| 5120×2880 (before, 4K) | 221.4, 219.3, 219.1, 234.2, 228.4 | 39.9, 39.9, 39.7, 42.4, 40.5 | 507.0, 496.0, 497.5, 515.2, 519.3 | 107,892, then 107,880 ×4 |
| 3840×2160 (after) | 161.5, 161.9, 160.3, 172.3, 168.2 | 22.3, 22.9, 22.2, 22.5, 23.6 | 426.2, 423.1, 419.6, 435.8, 433.0 | 88,908, then 88,860 ×4 |

Decoding (225–248 ms) and cropping (7.1–7.9 ms) are the same in every
row.

**End to end**: the release daemon on `scoot --headless --width W --height
H --outputs 1` (debug scoot, `[output] scale = 1.5`), 3 `set`s of the same
file over the socket from a Python client; CPU is the whole process's
`utime + stime` across the `set`; peak is `VmHWM` reset before it (the
first `set` has nothing on screen yet, the others hold the previous
buffer while drawing the next):

| Output | | reply, ms | CPU, ms | peak, kB |
|---|---|---|---|---|
| 1600×1000 | before | 372.9, 363.0, 410.1 | 370, 360, 400 | 78,572, 89,692, 89,576 |
| | after | 344.2, 334.8, 350.4 | 340, 330, 350 | 74,944, 81,372, 81,380 |
| 3840×2160 | before | 496.2, 494.9, 507.3 | 480, 500, 500 | 106,956, 164,748, 164,748 |
| | after | 430.2, 420.0, 434.4 | 410, 420, 440 | 87,976, 120,492, 120,536 |

Idle after a `set` at 1.5 (after, 1600×1000), 30 s: 0 context switches,
0 CPU ticks, 1 thread, `RssAnon` 484 kB. Binary (release, stripped):
1,500,008 → 1,508,200 B (+8,192).

After the review's fixes (release sha256 `cbaefb85…`, 1,512,296 B, +4,096
more), the same end-to-end run, 3 `set`s each: 1600×1000 buffer
6,410,404 B, reply 352.3, 358.1, 344.6 ms, CPU 340, 360, 330 ms, peak
74,928, 81,356, 81,492 kB; 3840×2160 buffer 33,177,600 B, reply 425.4,
454.6, 441.2 ms, CPU 420, 440, 430 ms, peak 88,032, 120,608, 120,592 kB.
The same buffers, and the times within the runs' spread: the mode check
is a few integer operations per draw decision.

### Not verified, and why

- **A real compositor without `wp_fractional_scale_v1` or
  `wp_viewporter`**: scoot and sway have both; the debug knob and the
  forced `full-shm` path stand in, on scoot.
- **The configure round trip's benefit**: removing it fails no test,
  because each batch of events arrived in one read on both compositors
  here and the worker's stale-size filter then drops the stale request
  before it runs. What it guards (a batch split across reads, one render
  wasted) could not be forced here; it is kept on the protocol's ordering
  guarantee, at one `wl_display.sync` per `configure`.
- **A `preferred_buffer_transform` other than `normal`**: no compositor
  here sends one for an shm buffer.
- **A 0×0 `configure`**: no compositor checked sends one for a surface
  anchored to all four edges; the model's sizing of it is unit-tested.
- **The extra render when a scale goes stale on sway** was seen in the
  trace (two buffers) but not timed.
- **`cargo nextest run --workspace`**: not run; no compositor code
  changed, and CI runs it. **`nix flake check`**, **actionlint** (not on
  this machine) and **macOS `cargo check`**: CI's.
- **`--tty`, the dev VM, a GPU**: not reachable from this container.

### For the next tickets

- [memory-and-idle-done.md](memory-and-idle-done.md): an image buffer at 1.5 is
  now the output's pixels plus at most a column and a row (1601×1001 on
  scoot: 0.16% off-screen); two outputs of one size still get two
  buffers.
- [lightest.md](../lightest.md): the table above is the fractional-scale
  row; a competitor drawing at `wl_output`'s integer scale holds 78% more
  pixels at 1.5. Pre-rotated buffers (`preferred_buffer_transform`) are
  the one scale-related saving left, for a software compositor with a
  rotated output that asks for it.
- [transitions.md](../transitions.md) and
  [animated-images.md](../animated-images.md): a frame is
  `Output::full_buffer`'s size; budget per frame at device pixels.
- [scoot-integration.md](../scoot-integration.md): nothing new; scoot's
  scale should be a multiple of 1/120 for exact drawing (the compositor
  item above).
