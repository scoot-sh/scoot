---
title: "While an input method holds its keyboard grab, every xdg_popup.grab is refused — no context menu opens in a text field with an IME running."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# While an input method holds its keyboard grab, every `xdg_popup.grab` is refused — no context menu opens in a text field with an IME running.

Found by independent review of `docs/backlog/resolved/xdg-popup-input-resolved.md`
(PR #44). Not a flexwm mistake — inherited from the same pattern Smithay's
own `anvil` reference compositor uses — but daily-driver relevant and
undocumented until now.

## The gap

`zwp_input_method_v2.grab_keyboard` installs Smithay's
`InputMethodKeyboardGrab` on the seat, with a compositor-generated serial
the client never sees, and it is released only when the client destroys its
`zwp_input_method_keyboard_grab_v2` object. A real IME (fcitx5's `waylandim`
module, confirmed by its own documented behavior) holds this grab for as
long as it is active, not just while composing.

`popup.rs`'s `grab_popup` checks, before installing anything, whether the
seat's keyboard or pointer is already grabbed by something this popup chain
is not nested inside (`taken`, in `grab_popup`) — deliberately, so a popup
grab can never steal a grab that belongs to someone else, such as a drag or
an IME mid-compose. The IME keyboard grab satisfies exactly this check: it
is a real, unrelated grab, so `taken` is true, so every `xdg_popup.grab`
request is refused for as long as the IME holds the seat.

Consequence: with fcitx5 (or any IME using the same grab-while-active
pattern) running, right-clicking for a context menu, or opening a combo box,
in *any* text field — not just the one the IME is composing in — gets no
menu at all, refused silently the same way every other outranked grab is.

## Why this isn't flexwm doing something wrong

The alternative — letting a popup grab pre-empt an IME's keyboard grab — is
worse: it would mean a menu opening anywhere could silently interrupt
in-progress composition in a completely unrelated text field, which is a
correctness regression an IME user would notice immediately and repeatedly.
flexwm's own check here is already *stricter* than anvil's equivalent
(`taken` is checked for both devices before either is touched, so a refusal
here can never leave half a grab installed, which anvil's reference
implementation can) — this entry is about the interaction between two
individually-correct pieces of machinery, not a bug in either.

## What it would take

Not attempted here — needs a real IME (fcitx5 or ibus with a Wayland
frontend) plus a real toolkit menu to even confirm the exact shape of the
interaction live, which wasn't available on the dev VM when this was found.
Candidate directions, once confirmed:

- Scope the IME grab's exclusivity to the surface it's actually composing
  in, so a popup grab request from a *different* window's context menu
  isn't blocked by an IME active somewhere else — would need Smithay
  support or a flexwm-side check keyed on which surface currently holds the
  active text-input field, not just "is the seat's keyboard grabbed at
  all".
- Or: confirm this is actually rare in practice (does fcitx5 really hold the
  grab continuously, or only during an active composition sequence?) before
  spending effort — the severity of this entry depends entirely on that,
  and it hasn't been measured.
