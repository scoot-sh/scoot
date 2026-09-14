---
title: "The first click on a fresh lock screen, before the mouse has moved, reaches nobody"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# The first click on a fresh lock screen, before the mouse has moved, reaches nobody

The first click on a fresh lock screen, before the mouse has moved,
reaches nobody (item 18, found by round two's hardware bug-bash rather
than by any test). `new_surface` does re-derive pointer focus, but it runs
while the lock surface is still *unmapped*, so the hit test finds nothing;
the commit that maps it asks for a render and nothing else
(`handlers.rs::commit` falls through `id_of` to `commit_layer_surface`,
which a lock surface is not). Measured on real `--tty`: the locker gets
`wl_keyboard.enter` at +2ms and `wl_pointer.enter` only at +4975ms, when
the pointer was first moved. Safe direction -- the click goes nowhere, never
to something behind the lock screen -- and a locker is a keyboard-first
thing, so this is comfort rather than correctness, but it is the same class
as the bug `refresh_pointer_focus` was added for. The fix is presumably to
re-derive on the commit that maps a lock surface, which needs a cheap way to
recognise one in `commit` without making every ordinary window's commit pay
for the lookup.
