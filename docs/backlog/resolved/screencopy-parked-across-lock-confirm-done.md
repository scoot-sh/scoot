---
title: "A screencopy frame parked for a session lock's blank is never re-armed once the vblank confirms — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A screencopy frame parked for a session lock's blank is never re-armed once the vblank confirms — DONE

## The entry as filed

> Found by a retrospective audit of PRs #73–#89 on 2026-09-17, traced against
> `673c9ea`. Introduced by PR #84, which moved lock confirmation out of
> `render()` and onto the DRM vblank under `--tty`.
>
> `service_captures` parks a pending screencopy frame while a lock is waiting
> for its blanked frame to reach scanout (`screencopy.rs:714`, gated on
> `SessionLock::awaiting_blank`) — correct on its own: a capture must not
> observe the pre-blank desktop.
>
> The frame ticker, though, keeps running only for three conditions
> (`headless.rs:914`):
>
> ```rust
> if state.needs_render || !state.pending_idle.is_empty() || state.shots_draining() {
> ```
>
> A parked screencopy frame is none of them, so the tick that rendered the
> blank falls through to `state.timer_armed = false` (`headless.rs:917`) and
> drops the timer. Confirmation then arrives from the DRM event handler via
> `note_flip_completed` (`session_lock.rs:1317`), or from the fallback timer
> via `note_blank_timeout` (`session_lock.rs:1333`) — and neither re-arms the
> ticker. `ensure_ticking` does not appear in `session_lock.rs` at all, and
> the DRM handler's own re-render is gated on `present_skipped`, which is
> false in the normal case.
>
> Net effect: once `awaiting_blank()` clears, nothing runs `service_captures`
> again, so the parked frame is neither delivered nor failed. Before #84,
> `confirm_lock` ran *inside* `render()`, so `service_captures` in that same
> tick already saw `awaiting_blank() == false` and delivered.
>
> Severity is bounded by what else re-arms the ticker: the locker's first
> surface commit does, so in the ordinary case the frame is late by
> milliseconds rather than lost. The indefinite case is a lock client that
> takes the lock and never commits a surface — the abandoned-lock path the
> lock code otherwise handles deliberately. `msg wait-idle` and `msg
> screenshot` are unaffected, since `pending_idle` and `shots_draining()` are
> both in the re-arm set above.
>
> Fix shape: one `ensure_ticking()` on the confirm path, so the tick that
> clears `awaiting_blank` is followed by one more. Worth deciding instead
> whether a parked capture belongs in `frame_tick`'s own condition set
> alongside `shots_draining()` — that is the version that cannot be forgotten
> again by the next thing that moves work out of the render tick, which is
> exactly how this arrived.
>
> Testing note: this is invisible to the existing suite because both the park
> and the confirm are exercised, just never in the same test — a regression
> test wants a pending capture *and* a lock confirming on a vblank, asserting
> the frame is delivered without any further client commit.

## Resolution

Decided **(a): re-arm the frame ticker on the confirm path** — and, on
independent review, the arm was hoisted from the two confirming branches
into `confirm_lock` itself, gated on actually taking a `pending`. The two
deferred paths (`note_flip_completed`, `note_blank_timeout`) arm exactly as
before, but so does any future wait-clearing path that calls
`confirm_lock`, by construction — which is the structural half of what
option (b) offered, without its cost. A vblank that matches nothing (stale
completion, previous frame, untrackable error) and a timeout polled before
its bound arm nothing, so non-confirming calls add zero behavior change.
Inside the render tail the arm is a no-op branch (`frame_tick` only runs
while the timer is armed); the only out-of-tick confirm (an out-of-tick
`render()` that confirms — today only the IPC-screenshot path) buys one
extra tick that finds nothing and drops itself.

**(b) — a parked capture in `frame_tick`'s re-arm set — was weighed and
rejected**, for a reason stronger than taste. The naive form (any parked or
due capture) directly contradicts the deliberate idle design documented at
`frame_tick` itself ("keeping the timer alive for it would undo the idle
behaviour `ensure_ticking` exists for"): a shell overview preview parked on
a static desktop would pin the timer at 60Hz forever. The scoped form
(parked *and* awaiting) avoids that but still burns up to ~60 wakeups/s
across a one-second fallback wait, couples `frame_tick` to lock-plus-capture
state, and buys nothing the re-arm doesn't already cover. The
re-forgettable objection stands, and is answered by the pin tests below, the
hoist into `confirm_lock` (future clearing paths inherit the re-arm), plus
a pointer comment at the park site (`screencopy.rs`, at the
`awaiting_blank` early return) naming the confirm as scheduling the
follow-up tick.

Two things the implementation trace established, both load-bearing for why
this shape is sufficient:

- No post-confirm render is needed. On `--tty` the blank tick already
  bumped `frame_serial` (making the parked frame due) and left the blank in
  the framebuffer; the re-armed tick's `render()` early-outs on the
  consumed dirty flag and `service_captures` delivers straight from those
  pixels. Content is the post-blank screen by construction of that
  ordering.
- No present-skip interaction. `drm_event` calls `note_flip_completed`
  first and `request_render` after, gated on `present_skipped`; both funnel
  into the idempotent `ensure_ticking` (`timer_armed` guard), so a
  confirming vblank alongside a skipped present arms once, not twice, and
  the skip's re-present still records through `await_vblank` on the next
  render tail exactly as before.

### Cost

Per confirmed lock: one `ensure_ticking` (a branch plus a timer insert when
the timer is down, which it is here) and one extra frame tick whose render
early-outs, whose `service_captures` walks an empty-or-not-due list, and
which then drops the timer. Steady state is proven, not asserted: every new
test ends with `settle()` plus `assert!(!timer_armed)` — the timer fires
only when work exists. No hot-path change: `frame_tick`'s re-arm set is
byte-identical, so per-frame behavior is untouched (per the ticket, no
hot-path benchmark beyond this reasoning).

### Edge dispositions

- Confirm with no parked capture: the re-arm still fires (unconditional in
  the confirming branch) — exactly one spurious tick, then idle. Pinned by
  `a_confirm_with_no_parked_capture_costs_one_tick`.
- True unlock-before-confirm is unreachable: Smithay routes
  `unlock_and_destroy` only once `locked` has been sent, which clears
  `pending` (see `SessionLockHandler::unlock`). The reachable shape is the
  locker dying mid-wait: the session stays locked (abandoned), the wait is
  still taken by the vblank/fallback, and the parked capture is delivered
  the locked framebuffer — pinned by
  `a_parked_capture_outlives_a_locker_that_dies_mid_wait` (byte-identical to
  the framebuffer, zero desktop pixels).
- Multiple parked captures across one confirm: one tick's
  `service_captures` serves every due session — pinned by
  `parked_captures_on_two_sessions_are_all_delivered_by_one_confirm`.
- Both confirm paths verified, not just vblank: the fallback has its own
  delivery test with its own re-arm pin.

### Tests

Five new tests in `screencopy/tests.rs` (plus a `Step::LockNoWait` client
step that locks and maps a surface without waiting for `locked`, keeping
all three proxies mapped — a deferred confirmation delivers an earthly
delay later, unlike `Step::Lock` whose delivery precedes the next client
roundtrip). Each drives the `--tty` shape without DRM hardware: backend
taken away so the lock's render confirms nothing, the `await_vblank`
record made by hand, `needs_render = false` plus a real tick to reproduce
the timer drop (asserted), backend restored, confirm driven by hand.

Fail-first, dev VM: all five fail pre-fix (each on its re-arm pin —
`timer_armed` immediately after the confirm call, with zero dispatches in
between so nothing but the confirm path could have armed it), all pass
post-fix. One headless honesty worth recording: the blank is never drawn
here, so the parked frame is not due at confirm time and the backdrop stays
stale — the delivering render in these tests is `refresh_lock_state`'s
catch-up on the first display dispatch, composed with the fix's re-arm.
Delivery alone therefore cannot discriminate headless (it happens pre-fix
too, found by backtrace while building these tests); the pin is the
regression test, and the delivery plus pixel assertions prove the re-armed
ticker serves a parked frame with post-blank content and goes idle. On
`--tty` the blank tick paints the backdrop, refresh stays quiet, and the
fix's tick is the only re-arm. `PollFrame`'s 300ms bound plus the harness's
10s patience mean no test can hang the suite.

### Verification

Full set green post-fix, dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`): `cargo test -p flexwm` (859 passed,
1 ignored), `cargo nextest run --workspace` (964 passed, 1 skipped),
`cargo clippy -p flexwm --all-targets -- -D warnings` clean,
`cargo fmt --check -p flexwm` clean (Mac-side), `scripts/smoke-test.sh`
17 ok, rc=0.

Not re-run live on `--tty` hardware: the change is backend-agnostic
(no DRM/present path touched — two calls added to already-covered confirm
functions), and the deferred-confirm shape is harness-simulated as above.
The one scenario only hardware could show (a real vblank confirming a real
blanked flip with a parked capture and no locker commit) is exactly what
the pin tests simulate at the method level.

No README change: no user-facing behavior changes — no new config, flag,
binding, or IPC surface. A parked capture arriving instead of hanging is a
bug fix within behavior the protocol already permits ("may wait an
indefinite amount of time"), and README's capture section documents bounds
and guarantees, none of which move.
