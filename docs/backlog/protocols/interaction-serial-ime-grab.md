---
title: "An IME keyboard grab makes the activation gate credit a client that received nothing."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# An IME keyboard grab makes the activation gate credit a client that received nothing.

Found by independent review of the `xdg-activation-v1` input-serial gate
(`docs/backlog/resolved/activation-serial-validation-done.md`). Not an
escalation and not a crash: the effect is a *divergence* between what
`input/interaction.rs` documents ("delivered" is meant literally) and what
actually happens while an input method holds the keyboard.

`zwp_input_method_manager_v2` is bindable by any client (`state.rs`'s
`InputMethodManagerState::new::<Self, _>(&dh, |_| true)` — same trust model as
the other privileged globals, see `README.md`'s trust note). When an IME calls
`grab_keyboard`, the pinned rev's `InputMethodKeyboardGrab::input`
(`src/wayland/input_method/input_method_keyboard_grab.rs:41`) sends the key to
the IME's own `zwp_input_method_keyboard_grab_v2` object and never touches the
inner handle, so the seat's keyboard focus is not written to at all.

flexwm's `key()` resolves the recipient as `keyboard.current_focus()`, so
while the grab is up:

- every key is recorded against the focused window's client, which received
  nothing;
- the client that did receive it — the IME — cannot spend it, since the entry
  names someone else.

Why it is not an escalation: the credited client is the window the user is
typing into, which is exactly who the gate means to credit. The user really
is interacting with it; the keystrokes are on their way to it as composed
text. So the practical outcome is the intended one, reached by an accident of
who the compositor thinks the recipient is.

What it would take: a decision about how "recorded recipient" should behave
under keyboard grabs in general, not a one-liner. `KeyboardHandle` exposes
`is_grabbed()`, so the cheap options are (a) don't record at all while a grab
is active — correct but loses the serial for a user who is typing through an
IME, which is the one case that most needs it — or (b) credit the grab's own
client, which is right for the IME but makes the focused window unable to
mint a token from a keypress the user aimed at it. The third option is to
teach the recipient lookup about grabs properly, which needs an upstream way
to ask a grab who it is delivering to.

Until then, `input/interaction.rs`'s module doc names this file as the one
place its "delivered" claim is not literal.
