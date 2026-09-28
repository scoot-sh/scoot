---
title: "XWayland: X windows are scale-aware (drawn at ceil(scale), toolkits told over XSETTINGS) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: X windows are scale-aware — RESOLVED

RESOLVED 2026-09-28 (branch `claude/scoot-backlog-issues-3rfkfv`). Split out
of [XWayland support](xwayland-support-done.md). At an `[output] scale`
above 1 the X server draws at `ceil(scale)` in its own pixels (lower where
the whole layout would not fit X's 16-bit coordinates at that: see
[The layout bound](#the-layout-bound)), toolkits are
told that scale over XSETTINGS, and a reload follows the scale
(`compositor/xwayland/scale.rs`; its module doc is the reference). The
shape the entry proposed held, with the differences below. One fork
change, found in review: Smithay's `XSettings::update` never flushed, so
the `READY`-time settings write could sit unsent and an X app started
right after read no scale (`tests/scale.rs` failed 2 runs in 20). The fork
flushes it (`035d447c`, see [docs/forks.md](../../forks.md)); 20 in 20
since. Everything else is on the fork rev before it (`e7130254`), and the
coordinate audit below holds on `035d447c`, which touches only
`src/xwayland/xwm/settings.rs`.

## What landed

- **The client scale is set at the spawn, not at `READY`.** XWayland binds
  `wl_output`/`xdg_output` and sizes its X screen from them before it
  signals `READY`; Smithay's output code re-sends sizes only on an output
  change, so a scale set at `READY` (anvil's order) left the X screen at
  the logical size -- measured: 200x200 X pixels on a 400-pixel output at
  scale 2 before the fix, 400x400 after
  (`an_x_window_draws_at_native_resolution_at_scale_2`).
- **Toolkits: XSETTINGS, not environment variables.**
  `Gdk/WindowScalingFactor = S`, `Xft/DPI = S·96·1024`,
  `Gdk/UnscaledDPI = 96·1024`, through Smithay's `X11Wm::set_xsettings`
  -- GNOME's settings-daemon names. Not `GDK_SCALE`/`QT_SCALE_FACTOR` in
  `State::spawn`'s environment: `GDK_SCALE` pins GTK against the live
  setting (a reload could never reach it), and `QT_SCALE_FACTOR` would
  scale a Qt app running on Wayland a second time (spawn cannot know which
  a child is). Not the `Xft.dpi` resource: Smithay's window manager cannot
  write the root's `RESOURCE_MANAGER`, and a second X connection from the
  compositor can block on its own server.
- **Measured with a real GTK 3 app over X** (mousepad 0.7.0, headless
  1600x1000, `target/debug/scoot --xwayland`): at scale 2 it draws at 2x
  (menu bar text and chrome exactly doubled, sharp); at 1.5 at 2 scaled
  down; at 1 as before. A mousepad started at 2 and reloaded
  2 → 1.5 → 1 → 2 gave screenshots **byte-identical (sha256)** to fresh
  sessions started at 1.5, 1 and 2 -- GTK re-renders live off XSETTINGS.
  Qt and Java are not measured (no Qt/Java app in this environment); both
  document reading these settings.
- **Coordinates.** Smithay converts every X coordinate by the client
  scale at the pinned rev (audited below), so scoot needed exactly one
  change: the X wire limits in `manage.rs` (`INT16` positions,
  `1..=32767` sizes) are X pixels, so their logical bounds divide by the
  scale (`x_limits`). Without it a client's `ConfigureRequest` for 65535
  was clamped to 32767 logical and reached the server as 65534 -- XWayland
  24.1 granted it (`an_oversized_configure_request_stays_in_x_range_at_scale_2`).
- **A scale-1 session is untouched**: the client scale is set to the 1.0
  it already was, no XSETTINGS entry is written
  (`a_scale_1_session_is_untouched` reads the settings property: empty),
  and `x_limits(1)` is the old bound.

## The fractional decision: `ceil`

Options were (a) X at 1 (the old behaviour: blurry upscale), (b) X at
`ceil(scale)` scaled down, (c) X at `floor`, and (d) X at the exact
fractional scale. Chose (b):

- **Cost on pixman** (`x11_scaled_composite_cost`, release, one 400x300
  logical X window at `[output] scale = 1.5` on a 1200-square output,
  re-composited every frame, 3 runs x 5 x 200 frames, median per frame):

  | X client scale | buffer | frame (median of 5, three runs) |
  |---|---|---|
  | 1 (a, old) | 400x300, 468 KiB | 688.6 / 754.2 / 717.5 µs |
  | 1.5 (d) | 600x450, 1054 KiB | 286.0 / 286.5 / 291.0 µs |
  | 2 (b, chosen) | 800x600, 1875 KiB | 684.2 / 852.1 / 706.5 µs |

  (b) costs what (a) did per frame -- both are a scaled pixman composite,
  whose cost follows the destination pixels -- at 4x the buffer memory
  (the X server's pixmap and the shared-memory buffer each hold it). (d)
  is a one-to-one copy, 2.4x cheaper.
- **Why not (d), the cheapest:** X toolkits scale by integers only, so at
  1.5 a GTK app draws at 1 (chrome two-thirds size, text sized only by
  DPI) or at 2 (oversized by a third); and a non-integer client scale puts
  every X coordinate through a rounding in both directions (an X position
  of 1 is logical 0.67), so X windows land between physical pixels and
  round trips can drift. (b) keeps X pixels an exact multiple of logical
  ones and leaves the fractional resampling to the renderer, where every
  `ceil`-rendering Wayland client already is.
- **Why not (c):** `floor` is (a) at every scale below 2 -- the blur this
  item exists to remove.
- Below 1 the X scale is 1 (`ceil(0.5) = 1`): X at scale 1, as before.

## Runtime scale changes

`[output] scale` reloads live. `State::refit_xwayland` (at first
`rescale_xwayland`, which ran before the outputs were re-advertised; now
after, see [The layout bound](#the-layout-bound) below) sets the new
client scale, re-advertises the outputs to XWayland in it, re-publishes
the toolkit settings
(at 1 too, so the old ones do not linger), and re-sends every managed X
window the configure it last had -- the same logical rectangle, in the new
X pixels (`configure_x11`'s logical-equality skip would otherwise leave
them at the old X size) -- through the wire clamp again at the new scale.
Found in self-review: a plain re-send (`configure(None)`) of a window
clamped at scale 1 to logical x = 20000 (a column scrolled far away, on a
workspace not shown so `apply()` never re-places it) reached the server at
40000 and wrapped to X = -25536
(`a_runtime_rescale_keeps_far_x_windows_in_x_range`: fails with
`configure(None)`, passes re-clamped at 32766). Geometry stays consistent throughout: Smithay
records a surface's client scale per commit, so a window is the same
logical size before and after it redraws
(`a_runtime_scale_change_rescales_open_x_windows`, 1 → 2 → 1). An app that
reads the scale once at startup keeps drawing at its old scale in the new
pixels until restarted. An override-redirect window open across the reload
cannot be configured by a window manager and Smithay's record of its
position is in the old logical pixels, so it is drawn (and hit) offset
until it moves or closes. Nothing crashes: a reload before `READY` sets
only the client scale (`READY` publishes the settings from the scale then);
after the server died, the settings write fails into a warning.

## The layout bound

Found after `829c368` landed, and a regression from it: X at
`ceil(scale)` multiplies the whole layout into X pixels, and X
coordinates are 16-bit. Measured on `829c368`/`f64ce64`
(`target/debug/scoot --headless --width 3840 --height 2160 --outputs 8
--xwayland` at `[output] scale = 1.25`, 24576 logical pixels wide): the X
screen was 49152 X pixels wide; an `xclock` scoot placed at logical x
21516 sat at X 32766 (`manage.rs`'s wire clamp pins logical x at 16383
at scale 2); `QueryPointer` over it answered root x -16385 and no child.
Drawing and Wayland-side input were right, but everything X does in root
coordinates (a menu or tooltip it places, `XdndPosition`, `USPosition`)
was wrong past X pixel 32767. At X scale 1 (before `829c368`) that layout
fit.

**Fix:** the X scale is the largest integer from 1 up to `ceil(scale)` at
which the union of every output's logical rectangle fits: right and
bottom edges times it at most 32767 (not the `CARD16` 65535 of the wire:
XWayland keeps the screen size in the server's `ScreenRec`, whose width
and height are `short` -- 24.1.13's `update_screen_size` assigns it
straight in), left and top (a negative origin) times it at least -32768
(`scale.rs`'s `fit_x_scale`). A layout that fits at its integer keeps it;
one that does not even at 1 keeps 1, the limit X always had, with a
warning. The choice is stored (`State::x11_fit`), so the per-configure
reads stay a field read, and is re-made by one function,
`State::refit_xwayland`, called at every site that changes the output
layout once the layout is final and before its `apply()`:
`headless::add_output_with` (startup's extra outputs are before the
spawn; a `--tty` hotplug), `State::remove_output` (an unplug),
`State::resize_output_of` (a `--tty` mode change, `--nested`'s host
resize through `resize_output`), the scale reload (inside
`rescale_outputs`, once the outputs are laid out at the new scale, since
the new layout's extent bounds it too), and the test-only
`add_output_without_backend`. In the three that repack or re-lay out, it
runs right after the layout is final, before `refresh_layer_zone` can
`apply()`. `init_named` (the first output)
runs before XWayland can exist, and the spawn chooses from the layout as
it is then (`State::adopt_x11_scale`). `wlr-output-management` applies
nothing (every configuration is refused), so it moves no output. When
the choice moves, `refit_xwayland` sets the client scale, re-advertises
every output (`change_current_state` with nothing changed: Smithay
re-sends an instance exactly what its client scale moved, so XWayland
gets new `xdg_output` sizes and positions and resizes its X screen;
other clients get a bare `wl_output.done`), re-publishes XSETTINGS and
re-sends every X window its configure re-clamped. The layout change
itself went out a moment earlier in the old client scale, so XWayland
briefly sizes its screen for that before the re-send (both queued in the
same dispatch). Logged at info once per change when X draws below the
integer, with the scale, the integer and the layout's size.

After the fix, same command and probe (`xscreen.py`, a raw X11 client),
working tree on `f64ce64`:

```
INFO ... X draws below the output scale's integer: the layout at that scale would be wider or taller than X can address (32767 pixels) scale=1 ceiling=2 width=24576 height=1728
X root 0x7a screen 24576 x 1728 mm 6502 x 457
mapped child 0x200008 x 21516 y 12 w 1518 h 1704
QueryPointer root 22000 800 child 0x200008
```

and reloaded to `scale = 2.0` (15360 logical, 30720 at 2: fits, so X
returns to 2): `screen 30720 x 2160`, the window at X `26904` (logical
13452), `QueryPointer root 27800 800 child 0x200008` with the pointer at
logical (13900, 400).

Tests, `compositor/xwayland/tests/scale_fit.rs` (live at scale 2 on
8192-pixel-wide outputs, 4096 logical, so every X size is exact). All
four live ones fail on `f64ce64`:

- `a_layout_too_wide_for_x_at_its_integer_draws_x_at_1` (five wide
  outputs, 20680 logical): `(X scale, X screen, XSETTINGS)` was
  `(2, (41360, 400), (Some(2), ..))`, want `(1, (20680, 200), (None, ..))`;
  then a window on the far output at X == logical, and `QueryPointer`
  naming it.
- `a_hotplug_across_the_bound_moves_the_x_scale_both_ways` (three wide
  outputs fit at 2; a fourth plugged in, then removed): X screen
  `(33168, 400)` after the plug, want `(16584, 200)`, and the window at
  `(24, 24, 164, 352)`, want `(12, 12, 82, 176)`; logged once.
- `a_resize_across_the_bound_moves_the_x_scale_both_ways` (the primary
  grown to 8192: 16384 logical, exactly 32768 at 2, one past): X screen
  `(32768, 400)`, want `(16384, 200)`.
- `a_reload_across_the_bound_moves_the_x_scale_both_ways` (2 → 1.5 keeps
  the integer 2 but grows the layout to 16653): the window stayed at
  `(24, 24, 232, 486)`, want `(12, 12, 116, 243)`.

Hermetic: `the_x_scale_is_the_largest_that_fits_x_coordinates` (edges
32767/32768 right and bottom, -32768/-32769 left and top, a ceiling below
1, stepping down one at a time, `i32` extremes, and the rectangle union);
new, so it could not compile on `f64ce64`.

## Coordinate audit (pinned fork `e7130254`)

Converted by Smithay through the client scale, so scoot reads logical and
writes logical:

- Outputs: `wl_output.scale` (sent as `ceil(integer / client)` = 1),
  `xdg_output` logical size and position (so the X screen is X pixels).
- Input: `wl_pointer` enter/motion/axis, `wl_touch`, relative motion,
  pointer gestures, pointer warp, tablet tool motion, cursor hotspots.
- Surfaces: attach offset, damage, input and opaque regions (`wl_region`),
  and the buffer's logical size (`SurfaceView` takes the commit's client
  scale), so `bbox`/`geometry` and hit-testing are logical.
- Window manager: `ConfigureRequest` (divided), `ConfigureNotify` and a
  new window's geometry (divided; `last_configure` is logical), every
  configure sent (multiplied), `WM_NORMAL_HINTS` min/max/base sizes,
  `_GTK_FRAME_EXTENTS`, `_NET_WM_OPAQUE_REGION`.
- Drag-and-drop: `XdndPosition` root coordinates (both the motion path and
  the status replay), and the XDND proxy, which covers the X screen in X
  pixels and follows `RandrScreenChangeNotify`. The fork's XDND commits
  add no coordinate of their own. Clipboard and primary carry none.

In scoot: `unmanaged.rs::rect_of` (logical `last_configure` position with
logical `bbox` size -- consistent), `manage.rs`'s `USPosition` (logical,
so a dialog keeps the X root position it computed:
`a_us_position_dialog_keeps_its_place_at_scale_2`), the wire clamps (fixed,
above), `moveresize.rs` (anchored on the press, reads no X coordinate),
`dnd.rs`/`selection.rs` (no coordinates), captures (the composited
framebuffer: `an_x_window_draws_at_native_resolution_at_scale_2` pins the
IPC screenshot and `ext-image-copy-capture` to it, stripes and all).

Not converted, and unreachable:

- `zwp_locked_pointer_v1.set_cursor_position_hint` is stored raw; scoot
  never reads a hint (`relative_pointer.rs`).
- `wl_surface.preferred_buffer_scale` (scoot's own `send_surface_state`
  calls) carries the output integer, not that over the client scale; it is
  sent only to a v6+ `wl_surface`, and XWayland 24.1 binds v4 (measured).
  If a later XWayland binds 6 and acts on it, that is the place to divide.

One rounding remains by construction: an override-redirect window at an
odd X position (41 at scale 2) is at logical 20.5, which Smithay rounds to
21 and scoot draws at physical 42 -- one physical pixel from where the
client put it. Input goes to the same surface-local point either way, so a
click still lands on what is drawn.

## Tests

`compositor/xwayland/tests/scale.rs`, live on `Harness::headless_scaled`
(400 physical pixels square, 200 logical at 2). Fail-first against
`a9c07c7` (the branch point): 10 of the 11 failed there (the scale-1 pin
passed, as it should):

- `an_x_window_draws_at_native_resolution_at_scale_2` -- X screen
  `(200, 200)` vs `(400, 400)`.
- `an_override_redirect_menu_lands_where_it_put_itself_at_scale_2` --
  logical rect `(40, 20, 50, 40)` vs `(20, 10, 25, 20)`.
- `a_us_position_dialog_keeps_its_place_at_scale_2` -- placed at
  `(100, 60)` vs `(50, 30)`.
- `a_wayland_drag_drops_onto_an_x_window_at_scale_2` -- `XdndPosition`
  was the logical point, not twice it.
- `toolkits_are_told_the_scale_at_scale_2` -- no settings.
- `a_fractional_scale_draws_x_at_the_integer_above` -- no settings.
- `an_ipc_click_lands_on_the_x_widget_under_it_at_scale_2`,
  `a_runtime_scale_change_rescales_open_x_windows`,
  `an_x_drag_finds_its_targets_at_scale_2` -- timed out waiting for the
  X window at its scaled placement (the X server never had it there). The
  drag test was then reworked for layout only (three columns do not fit
  200 logical pixels, so its drop target became an override-redirect
  window); that version was not re-run against `a9c07c7`, and waits on
  the same scaled placement first.
- `an_oversized_configure_request_stays_in_x_range_at_scale_2` -- failed
  on `a9c07c7` at the exact width only (the bug needs a client scale), and
  failed for the real reason (65534 past the limit) with the client scale
  landed and the clamp not yet scaled.

Added after: `a_runtime_rescale_keeps_far_x_windows_in_x_range` (fail-first
above). Hermetic: `x_wire_limits_shrink_by_the_x_scale`,
`the_x_scale_is_the_integer_above_and_toolkits_hear_it`.

## Not done

- The `Xft.dpi` X resource (see above): an app that reads only that takes
  `xrdb -merge` from the user.
- Per-output scale ([`per-output-scale-mode`](../core/per-output-scale-mode.md))
  would make this a per-screen problem X cannot express: one X server has
  one client scale.

**Follow-up (2026-09-28, with the user):** the memory cost of drawing X at
`ceil(scale)` at a fractional scale is documented (`docs/configuration.md`,
`[output] scale`), and an option to choose it is filed, defaulting to sharp:
[`xwayland-fractional-scale-choice.md`](../protocols/xwayland-fractional-scale-choice.md).
