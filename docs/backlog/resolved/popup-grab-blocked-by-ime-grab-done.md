---
title: "While an input method holds its keyboard grab, every xdg_popup.grab is refused — no context menu opens in a text field with an IME running — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# While an input method holds its keyboard grab, every `xdg_popup.grab` is refused — no context menu opens in a text field with an IME running — DONE.

## Resolution (2026-09-18)

Verify-first found the ticket half-open: the IME-first order (IME holds the
seat, popup asks) was refused and pinned, but the reverse order (popup holds
the seat, IME grabs) silently left a mapped menu with no keyboard — and the
precedence the code enforced was never written down where a user could find
it. Both are closed here; the per-surface IME-scoping design the ticket's
"Scoping" section asks for stays open exactly as filed (it needs a
Smithay-side design that does not exist today), and "no menu while the IME
holds the seat" stands as the documented behavior, not a bug to work around.

**The hole, reproduced in-harness before fixing:** with a grabbing popup
holding the keyboard, a real `grab_keyboard` through the protocol displaced
it — Smithay installs the IME grab with an unconditional `set_grab`, no
serial check and no hook back into the compositor — while the popups stayed
mapped: `popup_done` 0, the grab still held in `State`, the client's keyboard
focus still naming the menu, keys flowing to the IME's grab object instead.
A menu Escape cannot close. The new test failed exactly that way unfixed
(`popup_dones` 0 vs 1) and passes fixed.

**The fix, one check in the existing reap path:** `settle_popup_grab`
already runs on the display-source dispatch *after* the IME request that
displaces the grab, so no new hook into Smithay was needed — it now also
dismisses a live grab whose seat keyboard serial is neither its own nor its
nested previous (the mirror of the grant-time `taken` check), then
re-derives focus through the foreign grab. `dismiss_popup_grab` is
serial-guarded, so the IME's grab survives the dismissal, and the refresh
enters the window through it normally. Costs one `Option` check per dispatch
flush with no menu open; no allocation on any path.

**Why dismissal is the protocol-consistent answer**, cited from the
protocol XML rather than general knowledge:

- `xdg-shell` (`xdg_popup.grab`): "the top most grabbing popup will always
  have keyboard focus" during a grab, and "if the compositor denies the
  grab, the popup will be immediately dismissed" — with locking the screen
  named as a dismissal cause. A displaced-but-mapped menu violates the
  first sentence; dismissal is the blessed verb.
- `input-method-unstable-v2` (`grab_keyboard`): "the compositor *should*
  send all keyboard events on the seat to the grab holder" (should, not
  must, and it "may decide not to forward any particular event"), and "must
  not further process any event after it has been forwarded". Nothing there
  forbids refusing or dismissing the popup; the IME keeps every key either
  way.

**Composed edges, each pinned by a harness test:**

- *Focus after both dismiss:* IME takes the keyboard (menu dismissed,
  keyboard back on the window through the IME grab, keys reaching the IME
  alone), IME releases (window keeps the keyboard, typeable again, same
  serial re-grabs fine — the dismissal files no session-continuation stamp,
  like every other non-destroy dismissal).
- *Nested menus:* the takeover dismisses the whole chain (`popup_done` × 2,
  both colors gone from the framebuffer), covering the previous-serial arm.
- *Lock:* popup + IME + lock composes — the lock dismisses the grab and
  re-derives focus to nobody, keys reaching no client behind the lock (the
  IME's own diversion survives, by protocol design and harmlessly: it
  forwards only to the focused field, and the password guarantee was always
  about background clients, never the trusted input path).
- *Serial interplay:* a grab that cannot validate still wins nothing — the
  new path only ever ends a grab, never grants one, so PR #56's gate is
  untouched and un-bypassed.

**Tests:** two new in `layer_shell/tests/popup.rs`
(`an_ime_grab_while_a_popup_grabs_dismisses_the_menu`,
`locking_while_an_ime_holds_the_keyboard_leaves_keys_with_nobody`), the
first confirmed to fail unfixed. Precedence documented once in `popup.rs`'s
module doc (lock > exclusive layer > IME grab > popup grab) and in the
README's popup section plus the input-method section's new cross-pointer.

**Deliberately not changed:** denying the IME grab itself is unimplementable
at the pinned rev (Smithay's handler takes only the new id, with no deny
path), and unsetting the seat grab on lock would desync the IME (its object
would still believe it holds the keyboard) — the ticket's own "refused menu
is user-visible, desynced IME corrupts text" reasoning, applied symmetrically.

Original entry, left as written below.

---

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

## Measurement (2026-09-17): continuous-hold CONFIRMED live

The severity question is settled: **fcitx5 holds the keyboard grab for the
whole active span, idle or composing — not only during composition.** The
entry's severity stands as filed; the composition-only downgrade is ruled
out.

Setup (dev VM, nothing installed into it — all ephemeral `nix shell`
runs plus a scratch client in `/tmp`, since removed):
- fcitx5 5.1.21 (`/nix/store/aa1kc6wh58ky12lin4v7bcmd9lwbnvd7-fcitx5-5.1.21`),
  under `dbus-run-session`, `WAYLAND_DEBUG=1`, fresh `XDG_CONFIG_HOME`
  (default `keyboard-us` group, IME effectively off, plain Latin).
- sway 1.12 headless (`WLR_BACKENDS=headless WLR_RENDERER=pixman`,
  one `HEADLESS-1` output) — the IME-behavior half needs *any*
  `input-method-v2` compositor, not flexwm specifically; see the next
  section for why flexwm itself could not drive this.
- A scratch Rust text-input client (not committed): maps an `xdg_toplevel`,
  creates its `zwp_text_input_v3` *before* mapping so the compositor's
  `enter` has somewhere to go, waits for map + focus, then
  `enable()` + hold 12s idle + `disable()`.

Raw wire, from fcitx5's own `WAYLAND_DEBUG=1` log (object ids as logged):

```text
[10:27:12.216808] zwp_input_method_v2#16.activate()
[10:27:12.216843] zwp_input_method_v2#16.text_change_cause(0)
[10:27:12.216851] zwp_input_method_v2#16.done()
[10:27:12.216859] -> zwp_virtual_keyboard_manager_v1#14.create_virtual_keyboard(...)
[10:27:12.216867] -> zwp_input_method_v2#16.grab_keyboard(new id zwp_input_method_keyboard_grab_v2#20)
[10:27:12.217337] zwp_input_method_keyboard_grab_v2#20.keymap(0, fd 21, 0)
[10:27:12.217351] zwp_input_method_keyboard_grab_v2#20.repeat_info(25, 600)
[10:27:12.217358] zwp_input_method_keyboard_grab_v2#20.modifiers(8, 0, 0, 0, 0)
... 12.04s of nothing: no key, no commit_string, no set_preedit_string,
... no delete_surrounding_text anywhere in the session (grep count: 0) ...
[10:27:24.257499] zwp_input_method_v2#16.deactivate()
[10:27:24.257543] zwp_input_method_v2#16.text_change_cause(0)
[10:27:24.257551] zwp_input_method_v2#16.done()
[10:27:24.257560] -> zwp_input_method_keyboard_grab_v2#20.release()
```

Grabbed 59µs after `activate`, held 12s with zero composition traffic of
any kind, released 61µs after `deactivate`. The grab's lifetime is the
*active* span (focused + enabled text field), full stop.

Same behavior in fcitx5's own source at the exact version under test
(`5.1.21` tag, `src/frontend/waylandim/waylandimserverv2.cpp`):
`done`-after-`activate` calls `ic_->grabKeyboard()`; `done`-after-
`deactivate` runs `keyboardGrab_.reset()`; no composition path
(`commit_string`, preedit, `delete_surrounding_text`) touches the grab.
There is no code path that grabs on composition start or ungrabs on
composition end — composition-scoped holding is unimplementable without
rewriting the IME, so the live result is structural, not a config quirk.

## Why this was not driven on flexwm itself

Against flexwm `--headless`, the same fcitx5 binary binds
`zwp_input_method_manager_v2` and then never calls `get_input_method` —
no `activate` can ever arrive, so no grab, so nothing to measure. The
mechanism is in the source above: `WaylandIMServerV2::init()` completes
only when *both* `zwp_input_method_manager_v2` *and*
`zwp_virtual_keyboard_manager_v1` are present, and flexwm advertises no
virtual-keyboard global (confirmed against flexwm's live registry dump:
`zwp_input_method_manager_v2` and `zwp_text_input_manager_v3` present,
no `zwp_virtual_keyboard_manager_v1`).

Two consequences, both load-bearing for scope:

1. **On flexwm today the fcitx5 hole is unreachable**: fcitx5 can never get
   far enough to hold the seat, so no context menu is ever refused on its
   account. The `taken` refusal still fires for any other grab holder (a
   synthetic `grab_keyboard`, a held-button implicit grab) — pinned by the
   test below — but a real fcitx5 user hits nothing until flexwm
   advertises virtual-keyboard (or an IME without fcitx5's gate is used).
2. Advertising `zwp_virtual_keyboard_manager_v1` is a whole new protocol
   surface, not a measurement-harness trick — deliberately out of scope
   here. It is also the commit that *arms* this entry: whoever advertises
   it should re-read this entry first.

ibus was not driven: ibus never binds `input-method-v2` at all (its
Wayland path is client IM modules over D-Bus to the daemon, no seat grab),
so there is no ibus holding behavior to measure — the entry is fcitx5-only
in practice.

## Scoping: fix filed, not implemented

Deliberately no behavior change in the measuring PR. The tempting
relaxation — grant a popup grab from a surface other than the IME's active
one — is not small and not safe:

- Granting installs `PopupKeyboardGrab` via `set_grab`, which *replaces*
  the IME grab on the seat. The IME's grab *object* is not destroyed, so
  fcitx5 keeps believing it holds the keyboard (`hasKeyboardGrab()`
  stays true) while keys flow raw to the menu: repeat timers, pressed
  modifiers and composition state desync silently. A refused menu is
  user-visible; a desynced IME corrupts text.
- Keying the exemption on "actually composing" does not save it: the
  compositor can see preedit/commit traffic, but latin passthrough holds
  the grab while active without ever composing, so "not composing" still
  rips a live grab — and tracking per-surface composition state to split
  the difference is a feature, not a gate tweak.
- The one direction explicitly ruled out: letting a popup grab pre-empt
  an IME mid-compose. That stays forbidden (see "Why this isn't flexwm
  doing something wrong" above).

The follow-up, when it lands, needs a Smithay-side design that does not
exist today (scoped exclusivity or grab coexistence that keeps the IME's
object and the seat consistent with each other), plus per-surface
composition tracking — real design work, not a follow-up tweak. Priority
stays low: unreachable on flexwm until virtual-keyboard lands, and
correct-by-refusal until then.

## Compositor half, pinned

`a_popup_grab_is_refused_while_an_ime_holds_the_keyboard`
(`compositor/layer_shell/tests/popup.rs`, on the shared harness plus two
new steps, `ImeGrabKeyboard`/`ImeUngrabKeyboard`, driving a real
`grab_keyboard` through the protocol): while the grab is held the popup
grab is refused — refused at commit before any configure, `popup_done`
sent, keyboard never leaves the window — and the same serial grabs fine
the moment the IME releases, so a refusal files no session-continuation
stamp either. Confirmed to fail with the `taken` gate inverted
(`if !taken`), passing reverted. Incidental, measured while writing it:
dropping a `wayland-client` proxy sends *nothing* on the wire (no
`release`, seat stays grabbed) — the ungrab step calls `release()`
explicitly; the step's doc says so.
