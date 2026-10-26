# Slide animation: horizontal and vertical window slides (dev/design)

## Workspace-switch slide: the design decision

How a workspace switch reaches the renderer: `apply()` maps the core's
TARGET arrangement onto the Smithay `Space` — visible placements map,
invisible ones unmap. The frame path (`window_elements`) draws only windows
placed on that output; invisible windows are not drawn at all. Before this
ticket a switch snapped: the outgoing windows unmapped (their per-window
slides pruned as non-sliding) and the incoming windows had no old location
in the `Space` (unmapped → lookup misses → snap). No whole-viewport motion
existed.

Chosen shape: a single horizontal offset per output (viewport slide),
incoming-only. When the active workspace index on an output changes and the
new workspace shows a tiled window, that output's incoming strip slides in
from the output edge (`±` the logical width: right for a higher index, left
for a lower one) over the background, easing with the same
`duration_ms`/`easing` as window slides. Every incoming window shares the
one offset, so the strip arrives as one unit rather than many windows.

Why not draw the outgoing workspace too (a true cross-slide): it would need
retaining unmapped windows (or a snapshot list), drawing roughly twice the
windows per frame, damaging the full viewport per frame (old + new viewports
overlap everywhere), and holding both sets through the flight. Measured on
the CPU renderer the incoming-only slide costs one offset lookup per output
per frame plus the same per-window damage the moving windows already pay;
an outgoing-retaining slide would pay that plus a second strip's elements
every frame. The incoming-only shape costs nothing when off (two empty
`Vec`s, one branch per hot path, no timer/wakeup) and little when on
(frames only while in flight, damage limited to the incoming rects,
`needs_render` drops the instant the last viewport lands, per-output so
multi-output switches stay independent, lock/reload-disable snap, empty
workspaces snap because there is nothing to draw, single-window is one
window with the shared offset). `scoot-core` is untouched (TARGET
arrangement; IPC and hit-testing keep the target); one monotonic `Instant`
per output frame; no allocation on the per-frame path.

## Internals (short)

When `[animation] enabled = true`, tiled windows slide horizontally and
vertically to their new x/y instead of jumping. The core keeps returning
the TARGET arrangement; the compositor interpolates only where a window
is *drawn*.

- **Easing**: cheap closed-form ease-out cubic, `1-(1-p)^3` (two
  multiplies, no transcendental, no table), plus a `linear` spelling —
  and a `spring` spelling for niri-style motion. The spring is a 65-entry
  baked table (damping ratio 0.6: ~9% overshoot, settled back by the
  deadline): one lookup and a lerp per window per frame, no `exp`/`sin`
  on the frame path, no allocation. Progress is clamped `0..=1` before
  it reaches any curve, and every curve lands exactly at `1.0`, so a
  spring still snaps when the compositor falls behind and still goes
  fully idle at the end.
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
  or fade (clears + redraws the final frame). Duration/easing changes affect only
  slides and fades started after the reload (in-flight ones keep the timing they
  started with, so nothing jumps). Zero duration snaps.
- **Extreme sizes**: coordinates use `saturating_add`, so `i32::MAX`
  windows cannot wrap onto the screen.
- **Nix/IPC**: the Nix modules render free-form TOML (`tomlFormat.type`),
  so there is no typed option to mirror and no `nix/tests.nix` pin to
  add. There is no generic config/status IPC query today (`binds`,
  `outputs`, `windows`, ... are specific), so no IPC change.

## Open/close fade: the design decision

Behind the same `[animation]` gate as the slide, with no second knob:
new tiled windows fade `0 → 1` and closed tiled windows fade `1 → 0`
over the session's `duration_ms`/`easing`, reusing the slide's frame
scheduling (frames only while in flight, idle when done) and damage
discipline (fading rects only — Smithay's damage tracker keys alpha, so
an alpha change damages exactly the window's rects, never the full
output). No client re-commit: the fade only changes the opacity already-
committed buffers are drawn with. Rings snap (only content fades):
multiplying every ring segment's color per frame costs more than the
1px chrome is worth, and the content fade carries the effect.

Why fade only, no scale: a scale would resample every fading window's
pixels per frame (pixman scaling ≈ 2–3× a plain blit plus filtering),
grow and shrink the damaged bbox every frame, and tempt a re-commit
(size change → configure). A fade is one alpha multiply per window per
frame (the `render_elements_from_surface_tree` calls already take an
alpha), the damaged rect never moves, and no configure is ever sent.

Why the same gate, no second knob: the zero-cost-when-off bar already
covers both (one empty branch per hot path, no timer/wakeup), the
siblings (vertical, spring, viewport) all reused the gate, and a second
knob doubles the config surface for a 150 ms polish effect. A user who
wants slides without fades can ask for the knob later; the fade keeps
the gate's semantics (off by default, live reload, lock snaps).

What Smithay/scoot retain after the surface is gone, and the bound:
after `remove_window` the space no longer maps the window and the map
no longer names it, so without a retain there is nothing left to draw.
The close fade retains the [`Window`] itself (`Arc` clone: no pixel
copy, just the client's already-allocated buffers living up to one
`duration` longer). The hold is capped at 8 closing windows system-wide
(`MAX_CLOSING_FADES`): the 9th close evicts the oldest (it snaps away),
so even a client opening and closing windows in a loop pins at most 8
windows × 500 ms. If the surface is already dead (client disconnected),
its surface tree yields no elements and the fade draws nothing — it
still prunes on time, never leaks, never panics. `scoot-core` is
untouched (TARGET arrangement; IPC and hit-testing keep the target);
one monotonic `Instant` per output frame; no allocation on the
per-frame path (the closing pass is one `is_empty` branch plus a linear
scan over at most 8 entries).

Rules that fall out: open fades never restart on a move (the slide
retargets from the visual; the fade keeps its own clock); a
close-during-open starts from the open's in-flight alpha; a viewport
switch suppresses per-window fades (the strip rides the shared offset —
double animation would cost both every frame); hidden windows (workspace
switch, output move) never fade, only closed ones do; floating,
fullscreen and maximized windows snap shut; output removal drops that
output's fades with it; lock and reload-disable clear everything and
snap.
