---
title: "A window-focus change (e.g. a keybinding) does not dismiss an active popup grab, so `flexwm msg windows`' `focused` diverges from where keys actually go."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# A window-focus change (e.g. a keybinding) does not dismiss an active popup grab, so `flexwm msg windows`' `focused` diverges from where keys actually go.

Found by independent review of `docs/backlog/resolved/xdg-popup-input-resolved.md`
(PR #44). Deliberate and tested (`keybindings_still_fire_while_a_popup_grabs_the_keyboard`
asserts the menu keeps the keyboard after a `Super+h` focus-column
keybinding fires) — but the consequence is worth its own entry, because it
lands squarely on `CLAUDE.md`'s computer-use priority, not just daily-drive
polish.

## The gap

`PopupKeyboardGrab::set_focus` (Smithay's grab implementation) swallows any
`set_focus` call while the grab is live — that is what a grab *is*. flexwm's
own `state.focus` (the field `flexwm msg windows`' `focused` reports from)
*does* update when a keybinding like `Super+h` moves window focus, because
that field is compositor-internal state, independent of the seat's actual
keyboard focus. So after such a keybinding fires while a popup grab is
active:

- `flexwm msg windows` reports window B as `"focused": true`.
- Every real keystroke still goes to window A's popup — which, if the
  keybinding also scrolled the layout, may by then be off-screen.

Nothing is broken from the user's perspective (Escape or any click restores
normal focus), and this is squarely a case the module doc's precedence list
addressed on purpose: a grab is supposed to hold the keyboard through
everything but the lock/exclusive-layer/keybinding exceptions already
carved out. This entry is not asking to change that.

## Why it matters for computer use specifically

An agent driving flexwm through IPC has no way to observe "a popup grab is
active" — `flexwm msg windows` is the only focus signal exposed, and it is
wrong for the whole duration of a grab that outlives a focus-changing
action. A plausible agent sequence: query `windows`, see window B focused,
inject a keystroke meant for B — and it silently reaches A's popup instead,
which may not even be visible. No error, no signal, just the wrong target.
This is precisely the "targeting fidelity" gap
`docs/backlog/ipc/targeted-input-injection.md` already tracks in the
abstract; this entry is the concrete case a popup grab produces.

## What it would take

A real decision, not a one-liner — options in increasing order of
disruption:

1. **Dismiss the popup grab on any explicit focus-changing action**
   (`FocusColumn`, `FocusWindow`, `FocusWindowId`, workspace switches) the
   same way a session lock or an exclusive layer surface already does.
   Closest to how a real desktop behaves (opening another app's window
   while a menu is up usually closes the menu) but changes behavior for a
   keybinding user too, not just an agent.
2. **Expose grab state over IPC** (e.g. an extra field on the `windows`
   response, or a new `msg` request) so an agent can tell the difference
   between "window B is focused" and "window B is focused but a popup grab
   elsewhere holds the keyboard" without changing the grab's own semantics.
   Lower risk, but adds IPC surface for a corner case.
3. **Do nothing beyond documenting it** — the grab already ends on its own
   (Escape, a click, the client closing it) faster than most agent loops
   would notice, and the keybinding-still-fires guarantee is arguably more
   important to preserve than closing this window.

Whichever is chosen, `README.md`'s IPC/computer-use section should say so
explicitly either way, since this is exactly the kind of silent-wrong-target
failure that section exists to warn about.
