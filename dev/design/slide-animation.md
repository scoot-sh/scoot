# Slide animation: horizontal and vertical window slides (dev/design)

## Internals (short)

When `[animation] enabled = true`, tiled windows slide horizontally and
vertically to their new x/y instead of jumping. The core keeps returning
the TARGET arrangement; the compositor interpolates only where a window
is *drawn*.

- **Easing**: cheap closed-form ease-out cubic, `1-(1-p)^3` (two
  multiplies, no transcendental, no table), plus a `linear` spelling.
  Progress is clamped `0..=1` before it reaches the curve.
- **Duration**: `duration_ms`, default 150, clamped `0..=500`. Zero snaps
  (a slide that finishes on its first frame, no extra frame).
- **Time**: monotonic frame-clock `Instant::now()` sampled on `apply()`
  (slide start/retarget) and once per output frame in `draw_frame_with`.
  Never a separate timer thread. A late frame jumps its progress forward
  (frames are skipped, never queued); a frame past its deadline snaps.
- **State**: `State::slides: Vec<Slide>` (`{id, from_x, to_x, from_y, to_y,
  start, duration, easing}`), pooled in place (capacity retained, finished
  slides pruned, never rebuilt per frame; no `HashMap` rebuilt per frame).
  Empty whenever the option is off (cleared on disable, never filled) and
  whenever it is on but idle (finished slides pruned). Every hot path
  branches once on `is_empty` and otherwise runs the old code: no timer, no
  allocation, no per-frame work while empty.
- **Retarget**: `from_x`/`from_y` is the *visual* position at retarget time, so a
  new target arriving mid-slide continues from what is on screen, never
  jumps back. New windows (no old position) snap; windows that stop
  sliding (closed, hidden, floating, fullscreen, maximized) lose their
  slide, so nothing goes stale and nothing leaks.
- **What slides**: tiled, visible, non-floating, non-fullscreen,
  non-maximized windows only. Floating/fullscreen/maximized snap (they
  appear, disappear or resize in place). Invisible windows are not drawn.
  Width/height snap; only x/y interpolate (no size animation, no client
  re-commit: the slide moves already-committed buffers).
- **Damage**: Smithay's `OutputDamageTracker` sees moved element locations
  and damages old+new rects per frame; no full-output redraw, no extra
  damage. Only the moving windows' rects are damaged. The read-back copies
  the union bbox as before (small bbox for a slide on `--tty`).
- **Backends**: `--headless`, `--nested`, `--tty` all draw the same frame
  the same way (`draw_frame_with`); the slide is renderer-agnostic and
  rides `auto`/`cpu`/`gpu` equally (pixman is a first-class tier, not a
  fallback). The scanout tier gathers through the same
  `scanout_frame_elements` path with the same offsets. Captures
  (`scoot msg screenshot`, screencopy re-render) apply the same offsets,
  so a screenshot mid-slide shows the visual while `scoot msg windows`
  reports the final.
- **Frames**: `apply()` starts/retargets slides and `request_render()`s;
  `render()` prunes finished slides and keeps `needs_render` set while any
  remain, so the frame timer (`FRAME_INTERVAL` 16ms) re-arms for the next
  tick and drops itself the moment the last slide lands. The compositor
  goes fully idle the instant the slide ends. While locked, slides clear
  and no window draws.
- **Input**: pointer hit-testing (`window_element_under`), focus and
  `scoot msg windows` all read the TARGET (final) geometry throughout.
  The slide is purely cosmetic: clicking mid-slide hits the final
  position, matching what IPC reports. This is the least surprising for
  agents (which read `windows` to aim) and costs nothing (no hit-test
  change when disabled or enabled).
- **Reload**: `[animation]` applies live. Disabling snaps any live slide
  (clears + redraws the final frame). Duration/easing changes affect only
  slides started after the reload (in-flight slides keep the timing they
  started with, so nothing jumps). Zero duration snaps.
- **Extreme sizes**: coordinates use `saturating_add`, so `i32::MAX`
  windows cannot wrap onto the screen.
- **Nix/IPC**: the Nix modules render free-form TOML (`tomlFormat.type`),
  so there is no typed option to mirror and no `nix/tests.nix` pin to
  add. There is no generic config/status IPC query today (`binds`,
  `outputs`, `windows`, ... are specific), so no IPC change.
