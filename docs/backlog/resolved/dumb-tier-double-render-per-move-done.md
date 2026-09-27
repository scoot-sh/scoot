---
title: "Every pointer move renders twice ~50µs apart on the dumb tier — RESOLVED (measured, don't fix: no trailing render on current main)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Double-render per pointer move on the dumb tier — RESOLVED (don't fix)

RESOLVED 2026-09-27 (verdict-only PR, no code change). **Don't fix:
there is no trailing render to fix.** On current main every pointer move
schedules exactly one render — 275 render starts across both session
logs (32 + 243, including startup, cursor-blink, and screenshot-path
renders; the table below counts the 121 move-attributed renders across
five batteries), zero pairs under 2 ms apart (minimum inter-render gap
5.3 ms, screenshot-iteration-paced). The ticket's hypothesized present-skip /
vblank-retry cadence was measured directly and refuted as a pair source:
`VBlank` arrives with `needs_render == false` at every one of 60+
sampled completions, `present()` never skips in the move-only rigs, and
the retry arm never fires. The skip→vblank path itself is real — forced
with screenshots at a 5.5 ms iteration pace (flip still in flight at the
sync render → `present_skipped` → `vblank[needs_render=true]` →
`request_render`) — but it yields tick-paced (≥16 ms), retained-damage
re-renders that coalesce with the next move, never 50 µs damage-None
pairs. The damage-None renders that do exist (same-position moves) are
the move's own single render, verified free: 25–124 µs, no read-back, no
present, no flip, no damage-history advance. No README/protocols change:
no user-facing behavior changed.

## Mechanism (measured, dev VM vkms dumb/pixman release)

Rig: `scoot --tty --gpu /dev/dri/card1` (vkms Virtual-3, 1024x768,
`scanout="dumb"`, pixman), driven with `scootctl pointer move` exactly
like the #259 census rig, plus temporary `debug!` scheduler tracing
(`cursor_changed`, `request_render`/`request_cursor_render` with
`#[track_caller]` call sites, `ensure_ticking` arms, `frame_tick`
entries, per-render seq + `damaged`/`presented`/`retry` outcome,
`VBlank` settle outcome, tick keep-alive reason) — all removed after;
final tree is docs-only.

Per move, the trace is always the same four lines within ~15 µs, then
one tick ~17 ms later, then one render:

- `cursor_changed` → `request_cursor_render` (sole caller:
  `capture_cursor.rs:cursor_changed`, from `input.rs:move_pointer_to`)
  → `timer armed` → `frame tick` → `render start N` →
  `render end N[damaged, presented]` (wander) or `[!damaged,
  !presented]` (same-position).

No second scheduler fires: no `request_render` from any of the ~25 call
sites within 50 µs of a render, no `DrmEvent::Error` settle, no retry
re-arm. `needs_render` is a coalescing flag and the frame timer floors
spacing at 16 ms; the only synchronous `render()` caller on this tier is
the screenshot path (`screenshot.rs:capture_pixels_for`), which either
renders the move's own damage early (screenshot <16 ms after the move)
or early-returns (after the tick) — one `render_output` per move either
way, verified in the screenshot-interleaved battery.

Batteries (all single-output; `screenshot --output 2` refused, one
`driving this device` line):

| battery | moves | renders | pairs <2 ms | notes |
| - | - | - | - | - |
| empty desktop, 150 ms pace | 31 | 32 | 0 | +1 from two moves coalesced in one tick window |
| + foot, 150 ms pace | 23 | 27 | 0 | +3 startup, ~1 blink |
| + foot, 20 ms pace | 40 | 40 | 0 | every render damaged+presented |
| + foot, 300 ms pace | 12 | 12 | 0 | move-paced, ~308 ms apart |
| + foot, move+screenshot/iter | 10 | 10 | 0 | min gap 5.3 ms; skip→vblank observed here, coalesced |

## Cost (the ticket's question 2, answered for what exists)

- Same-position (damage-None) render: 25–124 µs each (n≈14), all
  `damaged_any=false presented_any=false`. By code (`draw_frame_with`):
  `render_output` takes Smithay's early return (no composite), the
  read-back arm is gated on `damage.is_some()` (no memcpy), `present()`
  never runs (no flip), `advance_generation` is gated on
  `history_advanced()` (no history motion, post-#259). Cost is element
  gathering + the damage walk + frame-callback dispatch.
- Damaged cursor render: ~0.4–1.3 ms steady state (7.1 ms first,
  full modeset path), composite + region memcpy + atomic flip on vkms.
- Jiffies, min-of-5, release: idle 0j/5 s (timer drops properly — five
  straight 0j samples); 30 wander moves 3j min (≈1 ms/move, one
  render+present each). No room in the budget for a hidden second
  render, and the traces show none.
- Idle note: render-less ticks seen mid-run are parked `wait-idle`
  waiters (`pending_idle=1`, completing after their quiet window) —
  by design, and idle settles to 0 jiffies. No follow-up.

## Residual unknown (recorded, not chased)

At #259 the implementer counted 9 (pre) / 18 (post) trailing Nones per
20 same-position moves via the Smithay damage trace, and the reviewer
corroborated pairs on both builds. That does not reproduce here across
five batteries. Units differ, which matters: those counts are per-move
damage *outcomes*, while only the reviewer measured sub-2 ms
*inter-render gaps* — both saw Nones, only one measured pairs. The
desync mechanism itself is ruled out as a pair source: pre-fix, 20
same-position moves produced 20 renders (11 incremental + 9
full-repaint), i.e. one render per move with outcomes alternating at
move pace — the desync has no synchronous second-render arm. No commit
between #259 (`62f8f32`) and this measurement (`f943789`) touches
render scheduling — the render-path commits are arrange-hoisting (#267,
which moves `arrange()` within the existing tick and adds/removes no
render), layer-callback withholding (#279, no layers in rig),
unplug-restore output add/remove, a spelling pass, plus the Smithay
repin `5b57532`→`74edbf3` (one functional delta: pixman `Repeat::Pad`
texture sampling, scheduling-irrelevant) — so a code change removing a
real pair is unlikely; counting methodology (trace spans/events vs
frames) or rig variance is likelier.
Re-verifying the old build was deliberately skipped: the verdict is
invariant to it (no mechanism, no cost, nothing to fix on main either
way). If a future rig reproduces pairs, reopen with the rig attached.

## Evidence

- Base: `f943789`, plus uncommitted `TEMP-MEASURE` debug tracing
  (removed after; final `git status` clean apart from this ticket move).
- Binaries: `/var/cargo-target/release/scoot`, `scootctl` (dev VM).
- Logs (dev VM): `/tmp/dbl-tty.log` (empty-desktop battery),
  `/tmp/dbl-tty2.log` (foot + fast + 308 ms + screenshot batteries).
- Analysis: timestamp deltas over the traced scheduler lines; pair
  scan over all 275 render starts (zero under 2 ms, min gap 5.3 ms).
- Seat released after (nothing running; `seatd` shows no clients).

## Original ticket

Filed 2026-09-26 from the PR #259 review (reviewer's own log
corroboration, not the implementer's report): every pointer move renders
twice ~50µs apart on both pre- and post-fix builds (reviewer: 18
sub-2ms trailing renders amid 0.308s-paced ones; pairs are the trailing
Nones). Pre-existing, identical pixels, no failure mode observed — the
PR's hypothesis is present-skip/vblank-retry cadence.

Measure first: what schedules the trailing render (present-skip retry?
vblank fallback? cursor-only damage re-walk?), and whether the second
render is genuinely free (damage-None, post-#259 no history advance, no
present) or carries hidden cost. File the mechanism with numbers; fix
only if the cost is real.
