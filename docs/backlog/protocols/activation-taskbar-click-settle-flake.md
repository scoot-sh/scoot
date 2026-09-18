---
title: "Flake: activation taskbar-click precondition misses under extreme parallel load"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Flake: activation taskbar-click precondition misses under extreme parallel load

Found 2026-09-18 while stressing the
[screencopy parked-poll flake fix](../rendering/screencopy-parked-poll-flake.md):
one full-binary run out of twelve failed in
`activation::tests::keyboard::an_activation_takes_the_keyboard_back_from_a_clicked_taskbar`
(`activation/tests/keyboard.rs:304`):

```
the click never reached the taskbar -- check TASKBAR_POINT against the layout
```

i.e. `drive_with_taskbar`'s `click()` at `TASKBAR_POINT` landed behind the
taskbar (`clicked_layer` stayed `None`).

The file already names this shape (line 283-286): the taskbar client's ack
proves the *client* finished its round trip, not that the compositor has
dispatched the buffer commit yet, so the click can run against a layer map
that does not have the taskbar in it yet. The existing single `settle()` was
not enough this once.

Load profile matters for reproducing: the failure appeared exactly once,
under deliberately abusive oversubscription — two full test binaries plus a
40-iteration targeted loop running concurrently on the dev VM (three
processes, each multithreaded) — and the same test passes 5/5 in isolation
on the same tree. It has never been observed under the standard suite
(`cargo nextest run --workspace`, ~15 consecutive green full runs across the
same session). A fix in the test's settle discipline (settle-until-mapped
rather than settle-once) is the likely shape; production behavior is not
suspected.
