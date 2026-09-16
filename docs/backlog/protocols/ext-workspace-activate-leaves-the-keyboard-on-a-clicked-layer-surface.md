---
title: "`ext-workspace-v1` workspace activation moves window focus but leaves the keyboard on a clicked layer surface"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# `ext-workspace-v1` workspace activation moves window focus but leaves the keyboard on a clicked layer surface

Found 2026-09-16, reviewing PR #53
(`resolved/activation-clicked-layer-keyboard-done.md`), which fixed the
identical bug in `xdg-activation-v1`'s `request_activation` and in the IPC
focus family, and left this caller untouched per ticket scope. The
independent reviewer verified it still calls `act` bare.

## The suspected gap

`State::clicked_layer` records the layer surface a click gave keyboard
focus to, and `layer_shell.rs`'s `layer_keyboard_focus` reads it: a
still-mapped surface with `on_demand` (or bottom/background `exclusive`)
keyboard interactivity named there **wins the keyboard** over whatever
window focus says.

`input.rs`'s `focus_under_pointer` is the reference implementation of
"focus this window", and it is two statements, in this order:

```rust
self.clicked_layer = None;
self.act(Action::FocusWindowId(id));
```

`ext_workspace.rs`'s `commit_workspace_requests` ends in the second
statement's sibling without the first:

```rust
self.act(Action::FocusWorkspaceIndex(index));
```

So: a panel that is an `on_demand` layer surface is clicked (it takes the
keyboard), the user picks a workspace from it, the panel switches
workspace over `ext-workspace-v1` — and the *window* focus moves while
every keystroke still goes to the panel. The same invisible mismatch PR
#50 and PR #53 fixed on their paths: `flexwm msg windows` reports the new
focus, nothing reports where the keystrokes go.

## Why this is suspected, not confirmed

Unlike the activation and IPC paths, no failing test demonstrates this
yet — nobody has put a real click, a real workspace switch, and a seat
keyboard-focus assertion together on this path. It is possible (though
unlikely) that workspace switching re-derives focus through a path that
does not consult `clicked_layer`, in which case there is no bug and this
entry should close as not-a-bug rather than collect a decorative clear.

## The fix, and what it needs

Probably one line (`self.clicked_layer = None;` before the `act`),
mirroring `foreign_toplevel_management.rs:591` and PR #53's
`activation.rs` — plus a test that really exercises it: a mapped
`on_demand` layer surface, a real pointer click on it, then a workspace
`activate` + `commit`, asserting the seat's keyboard focus follows window
focus. `foreign_toplevel_management/tests/requests.rs`'s
`taskbar_holding_the_keyboard` and
`activation/tests/keyboard.rs::a_refused_activation_while_locked_...` are
the shape — and a reminder that the weak version of this test
(hand-setting `clicked_layer` with no real layer surface in the fixture)
passes either way and proves nothing.

Also check in the same pass, rather than one at a time:

- Whether the `index == current.active` early return (already-active
  workspace) needs the click spent too — PR #50's reference fix spends it
  even on its already-focused fast path (`refresh_keyboard_focus` after
  clearing), on the grounds that the gesture happened regardless. If the
  test says the keyboard is already correct on that path, say so and move
  on; don't add code the test can't justify.
- Whether the locked-session ordering needs the same treatment PR #53's
  activation guard got (refuse before clearing, pinned by a locked test).
  `commit_workspace_requests` currently has no lock check at all — trace
  whether `act`'s gate is the only refusal and whether a refused switch
  would spend the click.

Why `medium`, not `high`: the agent-facing IPC half of this class is
already fixed (PR #53 clears `clicked_layer` for `FocusWorkspace` /
`FocusWorkspaceIndex` over IPC), so no agent loop silently mistypes; what
remains is the client-protocol half, where the panel that stays mapped
after switching is the exposure. Same invisibility, narrower blast radius.
