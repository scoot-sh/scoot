---
title: "An IME popup over a lock screen's password field is tracked but never drawn."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# An IME popup over a lock screen's password field is tracked but never drawn.

Found by review while implementing `text-input-v3`/`input-method-v2`
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). Not a crash and not
a regression -- before that work there was no IME at all -- but the one place
the feature does not do what the rest of it does.

An `ext-session-lock-v1` surface can hold a text field (that is what a lock
screen's password box is), and keyboard focus goes to it while locked, so
`zwp_text_input_v3` focus follows and an input method can be activated
against it. `State::parent_geometry` returns the default rectangle for such a
surface, which places the candidate window at the origin -- but that is moot,
because `headless.rs`'s locked branch **replaces the element list wholesale**
with `lock_elements(..)` rather than gathering popups, and `lock_post_frame`
sends no popup frame callbacks either. So the popup is tracked, positioned
and never rendered; an animated one would also be stalled for want of frame
callbacks.

Consequence: someone whose password needs an IME (a non-Latin passphrase)
gets no candidate window on the lock screen. Composition itself still works --
the text reaches the field -- so this is "cannot see what you are composing"
rather than "cannot type".

What it would take: gather `PopupManager::popups_for_surface` for each lock
surface in `lock_elements`, and send their frame callbacks alongside the lock
surfaces' own in `lock_post_frame`. The care needed is in `session_lock.rs`'s
own doctrine, not in the popup code: the locked render path is deliberately
"never gathered" rather than "drawn behind something opaque" (see that
module's doc), so adding a second source of elements to it has to preserve
that property -- an input-method client is not the lock client, and letting
an arbitrary client's surface render over the lock screen is exactly what
that design is refusing to do. Probably wants the popup restricted to the
lock client's own input method, or an explicit decision that an IME is
trusted while locked.
