---
title: "Popups open with the notification-card entrance"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-01"
---

# Popups open with the notification-card entrance

Filed 2026-10-31, child of GitHub epic
[#807](https://github.com/scoot-sh/scoot/issues/807) (section 4,
"scootbar (optional, later)"). Serves **daily-drive**: popups that fade
or spring in match the notification cards and the launcher panel
instead of appearing instantly.

Depends on: #807 (the epic), `bar-opacity-benchmark` (the opacity gate
for the whole slice), and `scootbar-depth-tokens` (shadow room: an
entrance that slides or scales needs the margin the shadow reserves, so
motion lands after depth).

## The gap

Notification cards animate in and out and restack on a spring through
the shared timeline helper (#811: `crates/scootui/src/motion.rs` —
`Curve`, `Timing`, `Settle`, driven from frame callbacks, reusing the
compositor's easing curves), and the launcher panel opens with a
scale-and-fade entrance (#814, and the dmenu panel in #838 stages its
frames through `stage_frame` in
`crates/scootlaunch/src/panel/staged.rs:102`). scootbar popups open
instantly: the five popup-capable modules — tray (`modules/tray/mod.rs:665`,
`wants_popup` at `:684`), network (`modules/network/mod.rs:2348`),
volume (`modules/volume/mod.rs:1200`), power (`modules/power/mod.rs:511`)
and looks (`modules/looks/mod.rs:377`), against the trait contract in
`modules/mod.rs:207-230` — draw through `crates/scootui/src/popup/`
(`layout.rs`, `paint.rs`, `interact.rs`, `hover.rs`, `wrap.rs`) with no
entrance token. Battery, bluetooth, brightness, clock, media,
microphone, window-title, workspaces, button, exec, push and custom
implement no `popup()` (verified 2026-10-31) and are out of scope.

## What to do

1. **Same entrance as the cards.** Popups open with the notification
   entrance (slide/fade/scale/drop with duration and bounce, set from
   the look through the shared tokens) driven by the shared timeline
   helper in `crates/scootui/src/motion.rs`, not a popup-local timer.
   Animations run only on frame callbacks, while something is moving,
   and leave no timers or redraws once idle (#807 Constraints, the way
   the compositor's `animation.rs` already behaves); `motion = "none"`
   costs nothing (no layers rasterized, no callbacks requested — the
   property `crates/scootnotify/examples/bench_motion.rs` measures for
   the card stack).
2. **Input region during motion.** While the popup moves, its input
   region moves with it every frame — each frame sends its own region
   (the cost `crates/scootnotify/src/popup/region.rs:1-11` documents:
   at rest every draw but the first after a change sends nothing; in
   motion every frame sends). Reuse the #835 pattern
   (`push_at_rest`, pooled rect lists, `stale`/`sent` bookkeeping in
   `region.rs:19-70`; `resolved/notify-input-region-follows-shape-done.md`)
   so clicks never land where the popup was a frame ago.
3. **Exit.** Either a matching exit animation or none, decided and
   stated explicitly: disappearing instantly while appearing with a
   spring is a visible mismatch the ticket must resolve one way or the
   other (the dmenu panel precedent from #838: no exit fade, a pick
   prints and exits at once — `resolved/dmenu-ignores-depth-and-motion-done.md`).
4. **Grab interaction.** Pin what a popup does to the keyboard/pointer
   grab while it animates: whether the grab is taken on first frame or
   on settle, and what a click outside or Escape does mid-entrance
   (dismiss at once; never strand input on a half-open surface).
5. Read the entrance from the same `[effects]`/`[apps.scootbar]` tokens
   as the depth work, with the same none-costs-nothing default, in the
   same validator/schema/docs set (`scootbar-depth-tokens` item 5).

## Acceptance

- All five popup surfaces (tray, network, volume, power, looks) open
  with the look's entrance; `motion = "none"` restores today's instant
  open byte for byte.
- No timers or redraws at idle once the entrance settles (measured, not
  asserted — the idle-wakeup standard from
  `resolved/second-wakeups-done.md`).
- Input-region-in-motion behavior pinned by test: clicks during the
  entrance hit the popup where it is drawn.
- Exit behavior (animate or instant) decided, implemented and
  documented; grab semantics during the entrance pinned by test.
- Golden coverage for the entrance styles; defaults unchanged.

## Test plan

- Golden/fingerprint tests unchanged at defaults (`motion = "none"` is
  the default); new goldens for the entrance frames.
- A live test on headless scoot opening each of the five popups, plus a
  CPU-tier screenshot mid-entrance and at rest.
- Warm-frame allocation check on the animated frames (layers cached,
  pooled rect lists, no per-frame allocation once warm).

## Not in this ticket

Which modules get popups (no new popup surfaces here), the shadow and
fill tokens themselves (`scootbar-depth-tokens` provides the room and
the tokens this entrance reuses), and the bar surface's own motion —
the bar is an exclusive-zone strip, not a card, and animating it is a
separate decision.

## Resolution (2026-11-01, verdict: PASS)

Popups open with a slide entrance (a drop away from the bar's edge)
through the shared timeline helper (`scootui::motion::Timing`), not a
popup-local timer. All five popup surfaces (tray, network, volume, power,
looks) share the same `Open`/`draw_open` path, so the mechanism is proven
once and holds for all; live headless opens power and looks (which have
popups to show without hardware), while tray/network/volume correctly
report no popup without a menu/wifi scan/sound server (evidence below).
Defaults stay instant and byte-identical; new goldens cover the entrance
frames.

### Exit: instant, matching the dmenu precedent

No exit animation. A pick (`closes`), a click outside (`PopupDone`), Escape,
a press on the bar (toggle), the module losing its popup, the output going
away, a scale change or a reload all close at once, even mid-entrance: `close`
destroys the popup before its surface (idempotent), releases the keyboard
with it, and drops a racing frame callback by id (`WlCallback Done` for a
closed id names nobody). This matches the dmenu panel precedent from #838
(`resolved/dmenu-ignores-depth-and-motion-done.md`: no exit fade, a pick
prints and exits at once so a script waiting on EOF never waits on motion).
An exit fade would hold a grab and input on a half-open surface past the
action that dismissed it; instant close never strands input. Pinned by the
existing popup integration tests (grab + outside closes + Escape in
`crates/scootbar/tests/popup.rs`) and by the frame-callback id check
(`daemon/popup/events.rs:375-412`).

### Grab: first frame, not settle; dismiss mid-entrance at once

The grab is taken on the press (first frame), not on settle: `build_popup`
grabs with the press serial before the first commit with no buffer
(`daemon/popup/mod.rs:707-728`), reusing the stored serial on reopen after
a resize, and takes the keyboard only while a grabbed popup is open (tooltips
and `invoke`-opened popups have none, so `invoke` has no Escape — closed by
invoking `popup` again or by anything that closes). A click outside or Escape
mid-entrance dismisses at once (see Exit); never strands input on a half-open
surface. Pinned by `popup.rs:431-432` (grab with the press serial before the
keyboard), `:666-667` (invoke has no grab/keyboard), `:860-909` (click opens
with grab, outside closes via `popup_done`), plus live `invoke` opens below
(agent path, no grab, still animates).

### Input region follows every frame, sent only when changed

While the popup slides, its region moves with it every frame — each frame
sends its own region; at rest every draw but the first after a change sends
nothing (the cost `scootnotify/src/popup/region.rs:1-11` documents). Reuses
the #835 pattern with pooled lists: `popup_rects` into `region_scratch`
(cleared, capacity kept), compared to `region_sent`, sent only when different,
then swapped (`daemon/popup/mod.rs:1448-1515`); no allocation once warm.
The popup draws at `drawn` (rest origin plus the slide `dy`), and pointer
hits subtract `drawn` to popup-local (`events.rs:192-220`), so clicks during
the entrance hit where the popup is drawn, never where it was a frame ago.
Pinned by `daemon/popup/tests.rs:the_input_region_follows_the_popup_and_keys_on_everything`
(different origins/sizes/radii give different rects; same gives same) and
`warm_entrance_frames_allocate_nothing_once_pooled` (0 allocations on 10 warm
animated frames).

### `motion = "none"` restores today's instant open byte for byte, costs nothing

`Timing::NONE` (the default: `motion = "none"`, `motion_ms = 250`,
`motion_bounce = 1.0`, so `Timing::new(None, _, _) = NONE`) keeps the surface
it always had: `motion_pad(NONE) = 0`, so the surface is content plus shadow
pad only, origin `(pad.left, pad.top)` as before; `needs_alpha` excludes
timing when inactive, `is_default` requires `!timing.active()`, so a square
opaque still popup paints via `popup::paint` exactly as before and everything
else via `paint_depth` with the same origin/shadow/fill/opacity; no `entered`,
no `surface.frame` request, no layers (repaint, not composite; shadow tables
cached as before, `none` builds nothing). Pinned by
`popup_motion/tests.rs:none_moves_nothing_and_needs_no_room`,
`a_zero_duration_is_no_motion`, `config/tests.rs:effects_wins...` (defaults
`NONE`, inactive), and all 1107 bin tests passing including the 21 snapshots
(defaults unchanged) plus 2 new entrance goldens.

### Animations run only on frame callbacks; idle costs nothing (measured)

The entrance starts on the first mapped draw (`entered = Some(now)`, once),
each frame draws at `entrance_dy(timing, start, now, edge)` (12 px slide,
top starts above rest and drops, bottom mirrors; spring overshoots past rest
briefly) and asks for the next `wl_surface.frame` while `!settled` and none
pending (one at most); the `Done` marks the next redraw while still moving
and none once settled, so an idle popup (settled or `none`) asks for nothing.
No timers. Measured the second-wakeups way (60 s windows from
`/proc/PID/status` voluntary+nonvoluntary, headless scoot 0.6.28 cached +
our cargo-built scootbar, CPU tier, 800x600, power popup open at rest):

- `spring-settled` (`motion = "spring"`, 300 ms, 0.6): start 45/0, end 45/0,
  wakes 0, voluntary delta 0, nonvoluntary 0.
- `none-rest` (same, `motion = "none"`): start 15/0, end 15/0, wakes 0.

Spring costs 30 extra wakes during startup+entrance (frame callbacks while
moving), then 0/60 s at idle once settled, same as `none`. Raw log
`/tmp/entrance-live/` on the VM (`/tmp/idle-entrance.sh`); commands below.

### Tokens, validator/schema/docs (same PR, per CLAUDE.md)

`scoot-look`: `BAR_DEPTH_KEYS += bar.motion, bar.motion-duration-ms,
bar.motion-bounce`, `user_effect_key("scootbar","effects",motion*)`,
`value_kind` (`Enum(MOTION_CURVES)`, `IntRange 0..=1000`, `Unit`),
`render_bar` emits the three when set (checked, omitted when unset).
`scootbar`: `[bar] motion/motion-duration-ms/motion-bounce` (`BarFile`,
`Config{ motion: Curve, motion_ms: u32, motion_bounce: f32 }`,
`Style{ timing: Timing }`, `timing()` via `Timing::new`; `[effects]` wins per
key over `[bar]` and the look layer through the same allowlist, refused loudly
for the file, reported+ignored for the layer). `look.rs` merges via `allows`
automatically. Schema `validate_effects` already gates the five tokens
generically; `validate_apps` via `check_app_value` now accepts the three for
`scootbar`. Site: `look-format.md` (bar reads all five, `[bar]` table, shell
table, `bar.toml` carries motion when set), `scootbar/configure.md` (Shape
+ Effects tables, slide/exit/grab/input/scale bullets), `theming.md` (layer
carries motion; radial-burst glow+spring). Pinned by `allowlist/tests.rs`
(depth+motion keys, user tables), `render/tests.rs:bar_depth...` (existing) +
config `effects_wins...` (motion wins, timing active, defaults `NONE`).

### Goldens, allocation, cache-key

Goldens: `popup-entrance-rest-1x-alpha`, `popup-entrance-mid-1x-alpha`
(non-black 0x80-gray theme, alpha plane: transparent 16 px entrance room,
soft shadow falloff, opaque popup at rest vs 6 px above mid; `mid != rest`
pinned, `top 16 rows = 0` at rest pinned). Blessed with
`SCOOTBAR_BLESS=1 cargo test -p scootbar --bin scootbar --
snapshots::tests::popup_entrance` on the VM; `no_snapshot_is_left_over`
passes (NAMES includes both). Defaults unchanged (1107 passed).

Allocation: `warm_entrance_frames_allocate_nothing_once_pooled`
(`scootbg_mem::count_allocations`, 0 on 10 warm animated frames,
pooled rects; shadow tables prepared once on open, read every redraw;
`none` builds nothing).

Cache-key: any cache keys on everything. Motion room keys the surface
(`motion_pad(NONE)=0` vs active 16, so active flip remakes via `new_pad !=
open.pad`); timing keys the restart (`style.timing != open.timing` restarts
with `entered=None`, colors-only keeps settled); input rects key on
origin/size/radius (`popup_rects` test); shadow tables rebuilt on restyle
when kept (`prepare` anew, same size new style). Pinned by
`motion_room_keys_the_surface_and_timing_keys_the_restart`,
`the_input_region_follows...`, plus restyle code
(`daemon/popup/mod.rs:898-1066`).

### How it was measured

Worktree `/tmp/wt-entrance` on `feat/scootbar-popup-entrance` from
`origin/main` `97248764e` (uncommitted when measured; VM tree
`/dev/shm/entrance/tree` tarred from it, `COPYFILE_DISABLE=1`, own
`CARGO_TARGET_DIR=/dev/shm/entrance/target`, `-j2`, incremental off for
gates after freeing 1.5G (`rm -rf .../incremental`); scratch deleted after
per brief on push). `cargo fmt` on the Mac; no VM restarts.

- `cargo test -p scoot-look` (Mac): 227 passed.
- `cargo nextest run -p scoot-look` (VM): 250 passed.
- `cargo clippy -p scootbar --all-targets -- -D warnings` (VM,
  `CARGO_INCREMENTAL=0`): clean (after fixing `Config::timing` dead code
  and const-assert lint).
- `cargo clippy -p scoot-look -p scootui --all-targets -- -D warnings` (VM): clean.
- `cargo nextest run -p scootbar --bin scootbar` (VM): 1107 passed,
  2 skipped (including 2 new goldens + `no_snapshot_is_left_over` +
  6 popup_motion + 2 popup region/alloc).
- `cargo nextest run -p scootui -p scoot-rt` (VM): 273 passed.
- `cargo test -p scootbar --bin scootbar warm_entrance...` (VM): pass (0 alloc).
- `SCOOTBAR_BLESS=1 cargo test -p scootbar --bin scootbar --
  snapshots::tests::popup_entrance` (VM): blessed 2 `.pgm` (80K each).
- `cargo fmt --all --check` (Mac): clean.
- `python3 scripts/backlog check` (Mac): clean (0 problems).
- Site: `nix build .#docs-site` not run locally (disk 98% full, pnpm tar
  fails with No space; CI docs-site job runs it).
- Headless live (VM, cached `scoot` 0.6.28 + cargo-built `scootbar` debug
  with motion, CPU tier, 800x600, Propo-Regular, `[bar] height=38 radius=14
  opacity=1.0 shadow=soft fill=solid motion=spring 300/0.6`, 5 modules
  placed): query shows power/looks/volume/network/tray started;
  `invoke power popup` ok, `invoke looks popup` ok,
  `volume` → "nothing on the bar to open from" (no server, correct),
  `network` → "has no popup to show now" (wired, correct),
  `tray` → "has no action `popup`" (opens via menu, no items headless,
  correct). Screenshots (CPU-tier, `scoot msg screenshot`):
  `power-mid-1x.png`, `power-rest-1x.png`, `looks-mid-1x.png`,
  `looks-rest-1x.png`, `power-mid-2x.png`, `power-rest-2x.png`,
  `looks-mid-2x.png`, `looks-rest-2x.png` (via `output-scale 1 2`, reopening
  after scale closes) in `/tmp/entrance-shots/` on the Mac; `cmp` mid vs
  rest differ (1x byte 4216 line 5, 2x byte 9400 line 20).
- Idle (same live, power popup open at rest, 60 s windows, fixed anchored
  `awk '/^voluntary.../'`): spring-settled 45→45 (0 wakes),
  none-rest 15→15 (0 wakes). Raw `/tmp/entrance-live/` + `/tmp/idle-entrance.sh`.

### Numbers (live, dev VM)

```
spring-settled 60s voluntary 45->45 (0) nonvoluntary 0->0 (0) wakes 0
none-rest      60s voluntary 15->15 (0) nonvoluntary 0->0 (0) wakes 0
```

Spring pays ~30 wakes during startup+300 ms entrance (frame callbacks while
moving), then 0/60 s at idle once settled, same as `none`.

Screenshots: `/tmp/entrance-shots/power-mid-1x.png`,
`/tmp/entrance-shots/power-rest-1x.png`,
`/tmp/entrance-shots/looks-mid-1x.png`,
`/tmp/entrance-shots/looks-rest-1x.png`,
`/tmp/entrance-shots/power-mid-2x.png`,
`/tmp/entrance-shots/power-rest-2x.png`,
`/tmp/entrance-shots/looks-mid-2x.png`,
`/tmp/entrance-shots/looks-rest-2x.png` on the Mac.

Epic #807 is closed; referenced, not touched.
