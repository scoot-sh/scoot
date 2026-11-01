---
title: "Popups open with the notification-card entrance"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootbar-depth-tokens"
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
