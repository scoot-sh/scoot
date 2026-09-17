---
title: "A window-focus change does not dismiss an active popup grab, so `focused` diverges from where keys go — RESOLVED (exposed over IPC, grab untouched)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A window-focus change does not dismiss an active popup grab, so `focused` diverges from where keys go — RESOLVED (exposed over IPC, grab untouched).

## The entry as filed

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

## Resolution (PR #55, merged 2026-09-16 as `4334d68`)

Option 2, by coordinator decision: dismissing the grab (option 1) would
overturn the deliberate `keybindings_still_fire_...` guarantee for human
users to serve agents — the wrong tradeoff for a daily-drive compositor —
and documenting alone (option 3) leaves agents guessing.

What shipped: `WindowSnapshot.popup_grab: bool` (`#[serde(default)]`, no
`PROTOCOL_VERSION` bump per the `icon`/`usable`/`locked` additive-field
precedent, wire tests both directions); `State::popup_grab_holder()` reads
the held grab's root via `keyboard_grab_start_data().focus` (stable for the
grab's life, unlike `current_grab`), gated on `!has_ended()`, mapped
through `State::id_of`; `window_snapshots()` sets the flag per window.
Grab semantics untouched — no diff on any install/dismiss/focus path, and
the keybinding guarantee passes unmodified. `README.md`'s IPC
computer-use section states the scenario, the agent rule, the asymmetric
`false` (layer-rooted grabs, locked sessions, old servers), and the
limits. Three new tests (holder alongside focus, layer-rooted all-false,
wire compat); full set green (657 / 753). Independent review came back
with no blocking findings and a live headless spot-check confirming
`"popup_grab": false` on the wire. Post-merge follow-up on `main`
(`f2f7c87`, comment-only): the ended-unreaped sliver the code comment
described is unreachable single-threaded, and the rustdoc now lists the
locked-session false the README already named.
