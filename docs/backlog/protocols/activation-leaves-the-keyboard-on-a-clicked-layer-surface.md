---
title: "`xdg-activation-v1` moves window focus but leaves the keyboard on a clicked layer surface"
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# `xdg-activation-v1` moves window focus but leaves the keyboard on a clicked layer surface

Found 2026-09-16, reviewing PR #50
(`resolved/wlr-foreign-toplevel-management-done.md`), which had the identical
bug in its own `activate` and fixed it there. Pre-existing in
`activation.rs`, untouched by that PR, and filed rather than fixed inside it
so the change lands with its own test and its own review.

## The gap

`State::clicked_layer` records the layer surface a click gave keyboard focus
to, and `layer_shell.rs`'s `layer_keyboard_focus` reads it: a still-mapped
surface with `on_demand` (or bottom/background `exclusive`) keyboard
interactivity named there **wins the keyboard** over whatever window focus
says.

`input.rs`'s `focus_under_pointer` is the reference implementation of "focus
this window", and it is two statements, in this order:

```rust
self.clicked_layer = None;
self.act(Action::FocusWindowId(id));
```

The clear is load-bearing, not tidiness — `set_focus`'s own doc says its
unconditional `refresh_keyboard_focus` is the only thing that takes the
keyboard back off such a surface, and that refresh cannot do it while
`clicked_layer` still names the surface.

`activation.rs`'s `request_activation` ends in the second statement without
the first:

```rust
tracing::debug!(?id, app_id = ?token_data.app_id, "activating a window");
self.act(Action::FocusWindowId(id));
```

So: a launcher that is an `on_demand` layer surface is clicked (it takes the
keyboard), the user picks an app, the launcher hands focus over with
`xdg-activation-v1` — and the *window* focus moves, the focus ring moves,
`flexwm msg windows` reports the new window as focused, while every keystroke
still goes to the launcher. Which is worse than either end of it: the user
cannot see where their typing is going.

## Why it has probably not been noticed

Most launchers unmap themselves immediately after activating something, and
`commit_layer_surface`'s unmap path re-derives focus, which clears the wrong
answer a moment later. It bites the launchers and taskbars that *stay*
mapped — and those are exactly the Quickshell panels this repo cares about.

## The fix, and what it needs

One line (`self.clicked_layer = None;` before the `act`), plus a test that
really exercises it: a mapped `on_demand` layer surface, a real pointer click
on it, then an activation, asserting the seat's keyboard focus is the
window's surface. `foreign_toplevel_management/tests/requests.rs`'s
`taskbar_holding_the_keyboard` is the shape — and a reminder that the weak
version of this test (hand-setting `keyboard_on_layer` with no real layer
surface in the fixture) passes either way and proves nothing, which is how
PR #50's first attempt at it got through.

Worth checking the other callers of `Action::FocusWindowId` in the same pass
rather than one at a time: as of writing they are `input.rs` (correct),
`activation.rs` (this entry), `foreign_toplevel_management.rs` (correct since
PR #50) — and a fourth, missed in an earlier draft of this entry:
`ipc.rs`'s `Request::Action` handler, which calls `self.act(Action::from(action))`
for every IPC action, `focus-window-id` included, with no `clicked_layer`
clear anywhere on that path.

That fourth caller is why this entry's priority is `high`, not `medium`: an
`xdg-activation-v1` launcher usually unmaps itself right after activating
something, which is the whole reason this bug went unnoticed (see above) —
but `flexwm msg action focus-window-id N` has no such self-correcting
mitigation, and it is the primary way an agent focuses a window over IPC.
An agent that clicks an `on_demand` panel (taking the keyboard), then drives
focus with `focus-window-id` and injects keystrokes, gets every keystroke
delivered to the panel instead of the window — with `flexwm msg windows`
reporting the window as focused the whole time, so there is nothing to
detect the mismatch from. This sits squarely on the computer-use mission
`CLAUDE.md` names as one of two things this project has to get right.
