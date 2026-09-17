---
title: "The pointer starts at the output's origin, so the cursor sits wedged in the top-left corner until the first motion — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The pointer starts at the output's origin, so the cursor sits wedged in the top-left corner until the first motion — DONE

RESOLVED 2026-09-17. The pointer is now centred on the output once, at
startup, through the quiet path. The three decisions the ticket named are
answered as the coordinator directed: centre on the one output at startup
only (read off the output being initialized, in logical coordinates, so a
future second output or a scale other than 1 cannot silently misplace it);
the placement is not pointer motion for idle/activation purposes (it goes
through `pointer_move_quietly`, never the announcing path); a VT-switch
reactivation leaves the pointer where the user left it (no re-place on any
reactivation or resize path).

Original entry, kept verbatim below.

Nothing places the pointer at startup, so Smithay's seat leaves it at (0,0)
and `--tty` — the one backend that draws a cursor — draws the arrow in the
extreme top-left corner of the screen, hotspot exactly on the corner, until
the user moves the mouse. Centring it on an output is the more familiar
behaviour, and the likely fix, but which compositors actually do that has not
been checked against their source here — don't take it from this entry.

Purely cosmetic, and not a correctness bug: a cursor has to be *somewhere*
before the first motion event. Found while resolving
[`tty-background-not-painted-done.md`](../resolved/tty-background-not-painted-done.md),
where it was the reason a corner pixel the smoke test sampled as "background"
read as the cursor's black outline instead. That test now parks the pointer
before capturing, so this is no longer blocking anything.

Worth an item of its own rather than a drive-by, because "centre it" has real
decisions in it: on which output once there is more than one; whether the
placement counts as pointer motion for idle/activation purposes (it must not
— see `pointer_move_quietly`'s comment on exactly that distinction for
`refresh_pointer_focus`); and whether a `--tty` session that reactivates after
a VT switch re-places it or leaves it where the user left it.

## What changed

`State::place_pointer_at_output_centre` (`compositor/input.rs`), called
once from `headless::init_named` — the one init point all three backends
reach, so headless and nested place it too even though only `--tty` draws
a cursor today. It reads the logical extent off the output itself via
`output_scale::logical_size` (the same rectangle the core and the `Space`
lay out in) and halves it, then goes through `pointer_move_quietly`:
no `announce_activity` (no idle-timer reset), and with no surface under
the pointer at startup, no focus derived and no interaction serial
recorded (nothing spendable for activation or a popup grab).

Deliberately not repeated anywhere else: `resize_output` (the path a
VT-switch reactivation takes when the mode changed, via
`session_event`'s reactivation arm and `Reconfigured::finish`) never
touches the pointer, and neither does the reactivation arm itself — a
failing reactivation probe cannot move the cursor out from under the
user's recovery keystrokes.

## Calls made and not made

- The smoke test's park-the-pointer workaround stays as is. With
  centring landed it is redundant for the background sample (the cursor
  now starts 600+ pixels away on every backend size the script uses), but
  it is harmless — an explicit `pointer move` plus `wait-idle` that still
  passes on both modes — and it protects the sample against future cursor
  changes rather than encoding today's placement. Removing working test
  code to celebrate a redundant fix would be churn.
- No resize clamping: after a shrink the pointer can sit outside the new
  rect until the next motion, which matches the existing absolute-move
  semantics (`Request::PointerMove` never clamps either) — stated, not
  changed.
- Zero-size output is unreachable, so no code guards it: `--width` /
  `--height` refuse 0 at parse (`cli.rs`), a DRM mode is never 0x0, and a
  0x0 nested configure means "keep the started size". The `None` arm
  (`unwrap_or((0, 0))`) is the same defensive fallback
  `pointer_move_relative` already uses.
- No README change: no config option, keybinding, CLI flag or IPC
  surface changed, and the startup cursor position is not documented
  anywhere to update.
- No hot-path benchmark: the placement runs once per process at startup,
  not per event or per frame.

## Evidence

Harness position assertions plus a live `--tty` screenshot, all against
the code as committed (see the PR description for the SHA and the raw
runs):

- Fail-first: `a_fresh_session_starts_with_the_pointer_at_the_output_centre`
  read `(0.0, 0.0)` before the fix, `(100.0, 100.0)` on a 200x200
  headless output after; it also pins no focus derived, no interaction
  serial minted, and normal motion afterwards.
- `a_later_resize_leaves_the_pointer_where_the_user_left_it` pins the
  resize path the VT reactivation shares. A true reactivation cannot be
  simulated in the harness (it needs a DRM device, libinput and a
  session), so that half is pinned here and verified by reading every
  statement of the reactivation arm — none touches the pointer.
- `startup_placement_centres_on_the_logical_extent` pins the
  logical-vs-physical distinction: at scale 2 on a 200px framebuffer the
  pointer lands at (50, 50), not (100, 100).
- Live `--tty` on the dev VM (1280x720 mode): a fresh session with no
  clients and no pointer motion screenshots the cursor's own bitmap shape
  at the centre — `(640,360)` and `(643,363)` read the black outline,
  `(648,360)` reads the background through the transparent region,
  `(3,3)` reads the background (artifact `/tmp/pointer-centre-proof.png`
  on the dev VM).
- `cargo test -p flexwm`, `cargo nextest run --workspace`, `cargo clippy
  -p flexwm --all-targets -- -D warnings`, `cargo fmt --check -p flexwm`
  all clean; `scripts/smoke-test.sh` green under `--headless` and
  `--tty` (EXIT=0 both).
