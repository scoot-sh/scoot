---
title: "An IME keyboard grab makes the activation gate credit a client that received nothing — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# An IME keyboard grab makes the activation gate credit a client that received nothing — DONE

Found by independent review of the `xdg-activation-v1` input-serial gate
(`docs/backlog/resolved/activation-serial-validation-done.md`). Resolved as
**decide + pin, no behavior change**: the current behavior is the intended
outcome, so this item records the decision and pins it with a regression
test rather than changing what is recorded.

## The decision

The ticket framed three options; all three are worse than the status quo:

- (a) Not recording under a grab would strip token-minting from users typing
  through an IME — exactly the population that most needs it (daily-drive
  harm for CJK users).
- (b) Crediting the grab's client would strip it from the focused window the
  user is actually interacting with.
- (c) Teaching the recipient lookup about grabs properly needs an upstream
  grab-identity API that does not exist.

So the recorded recipient under a keyboard grab stays the focused window: the
interacting party, receiving the keystrokes as composed text, which is who
the gate means to credit.

## The premise, verified against the pinned rev

The ticket's mechanism claim holds at the pinned Smithay rev
(`0ff00983b6007257a7a161a4fe8b14a778e2ac8f`):

- `InputMethodKeyboardGrab::input`
  (`src/wayland/input_method/input_method_keyboard_grab.rs:41`) sends the key
  to the IME's own `zwp_input_method_keyboard_grab_v2` object and never
  touches the inner handle (`_handle`, `_data` — both unnamed), so the seat's
  keyboard focus is not written to at all.
- `KeyboardHandle::current_focus` (`src/input/keyboard/mod.rs:1459`) reads
  that same inner focus, so flexwm's `State::key` recipient lookup keeps
  naming the focused window while the grab is up.
- The grab itself is installed unconditionally by the `GrabKeyboard` request
  (`src/wayland/input_method/input_method_handle.rs:276`, via
  `KeyboardHandle::set_grab` with no serial check), so a test client can hold
  a real one at will.

No re-scope was needed: the rev matches the ticket's description exactly.

## What landed

Two harness tests in `compositor/activation/tests/ime_grab.rs`, built on
this suite's own `drive` plus a second client that binds
`zwp_input_method_manager_v2` and holds a real `grab_keyboard` — an IME is
always somebody else's client, which is what makes the client assertions
non-vacuous:

- `a_key_under_an_ime_grab_mints_a_token_for_the_focused_window_and_nothing_else`:
  with `is_grabbed()` asserted on the seat, a press and release through
  `State::key` are `contains`-spendable by the focused window's client for
  both halves, `token_created` accepts the window's token and refuses the
  IME's for the same serial, and both halves are observed arriving on the
  IME's grab object (press + release, exactly two `key` events) — the grab
  diverts delivery while the recording stays with the window.
- `a_token_minted_under_an_ime_grab_still_activates_on_redemption`:
  creation plus redemption — the token minted from the grabbed keypress is
  accepted and, redeemed against the first window, moves focus there.

Both confirmed to fail unfixed before being kept: with the option-(a)
early-return temporarily added to `State::key` (`!keyboard.is_grabbed()` on
the recording condition), the ring never moves under the grab and both tests
fail in `press_a_key`'s "the keypress recorded nothing" guard — which is
what proves they pin the decision rather than decorating it. Probe reverted
afterwards; `input.rs` is byte-identical to before.

`input/interaction.rs`'s module doc is sharpened from exception-note to
decided-semantics: the grab paragraph now states the focused window is
credited deliberately, why (a)/(b) were rejected and (c) is unavailable, and
names the test that pins it.

## What this deliberately does not cover

- No behavior change to recording, activation, or IME handling — by decision,
  not by omission.
- No benchmark: the recording path (`State::key`'s tail) is byte-identical,
  so there is no before/after to measure.
- No README change: no user-facing behavior changes — no config option, no
  keybinding, no CLI flag, no IPC or protocol surface. Stated explicitly
  rather than silently skipped.
- `scripts/smoke-test.sh` not run: the diff is a regression test plus a
  module doc and cannot affect what it exercises. Stated, not assumed.
