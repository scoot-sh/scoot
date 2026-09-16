---
title: "`xdg-activation-v1` moves window focus but leaves the keyboard on a clicked layer surface — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `xdg-activation-v1` moves window focus but leaves the keyboard on a clicked layer surface — RESOLVED.

## The entry as filed

Found 2026-09-16, reviewing PR #50
(`resolved/wlr-foreign-toplevel-management-done.md`), which had the identical
bug in its own `activate` and fixed it there. Pre-existing in
`activation.rs`, untouched by that PR, and filed rather than fixed inside it
so the change lands with its own test and its own review.

### The gap

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

### Why it has probably not been noticed

Most launchers unmap themselves immediately after activating something, and
`commit_layer_surface`'s unmap path re-derives focus, which clears the wrong
answer a moment later. It bites the launchers and taskbars that *stay*
mapped — and those are exactly the Quickshell panels this repo cares about.

### The fix, and what it needs

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

## Resolution (2026-09-16)

Both callers fixed in one PR, each with a test that was confirmed to fail
against the unfixed code before being kept.

### 1. `activation.rs`: the one line, plus a lock gate the one line requires

`request_activation` clears `clicked_layer` immediately before its
`act(FocusWindowId)`, mirroring `foreign_toplevel_management.rs:591`
(including the comment style) and `input.rs`'s `focus_under_pointer`. And
because that clear is new, a locked session is now refused *before* it rather
than left to `act`'s own gate: pre-fix, a locked activation was refused with
nothing disturbed; clearing first and refusing second would have spent the
taskbar's click on a request that went nowhere, so the session would not
have come back as the user left it. The check is ordered with the existing
refusals (expired token, non-window surface), `act`'s gate stays as the
backstop `shell.rs` describes it as, and the doc comment says all of this
the way `wlr_toplevel_activate`'s does.

### 2. `ipc.rs`: the whole focus family spends the click, nothing else does

`Request::Action` clears `clicked_layer` when the incoming action is one
whose purpose is moving window focus — all five `Focus*` variants
(`FocusColumn`, `FocusWindow`, `FocusWindowId`, `FocusWorkspace`,
`FocusWorkspaceIndex`) — after the existing session-lock refusal and before
`act`. The boundary is deliberate and pinned by a test: layout and lifecycle
actions (`MoveColumn`, `CloseFocused`, `CycleColumnWidth`, …) change
arrangement rather than where focus is reported to be, so they leave a
deliberate keyboard placement alone. `CloseFocused` is the closest call — its
focus change is a side effect of a window dying, not a focus gesture — and it
stays on the not-spending side with the rest. Every `act` ends in
`refresh_keyboard_focus`, which is what consults `clicked_layer`, so the
question was never which actions *reach* that path (all of them) but which
ones *mean* "the keyboard should be on a window now".

### What this deliberately does not touch

- `ext_workspace.rs:369` (`FocusWorkspaceIndex` from an `ext-workspace-v1`
  activate) has the same shape — a workspace switch requested from a clicked
  panel would leave the keyboard behind it. Out of scope for this diff by the
  ticket's own scoping; filed separately.
- Keybindings (`input.rs:450`) go through `act` with no clear, as before. A
  keybinding is a keystroke delivered to wherever the keyboard visibly is,
  not an invisible focus report change — pressing a hotkey while typing in a
  panel's search field keeps typing in the panel, which the user can see.
- No `PROTOCOL_VERSION` bump and no `README.md` change: this is a bug fix
  with no new user-facing behavior — no new request, action, flag, binding
  or config. (Stated explicitly rather than silently skipped.)

### Tests

Three new tests, each on a real mapped `on_demand` layer surface clicked
through the real pointer, asserting on the seat's actual keyboard focus
surface — never on a hand-set field:

- `activation/tests/keyboard.rs`: two clients the way the real gesture
  works (one maps the windows and redeems the token through this suite's
  untouched script, the other is the taskbar). Key press lands while window
  2 holds the keyboard, real click moves it to the taskbar, the client's own
  activation moves window focus to window 1; asserts keyboard, `focus`,
  `keyboard_on_layer` and `clicked_layer` all agree on window 1.
- `ipc/tests/actions.rs::focusing_over_ipc_...`: all five covered actions in
  a loop — click, `handle_request(Request::Action(..))`, assert the keyboard
  is on whatever window focus reports (or on nothing, when stepping off the
  last workspace legitimately empties focus).
- `ipc/tests/actions.rs::a_layout_action_...`: the boundary —
  `CycleColumnWidth` after a real click leaves the keyboard on exactly the
  clicked surface, focus unmoved, the click unspent. Passes with and without
  the fix, by design: it pins what the predicate must *not* do.

Both suites live in their own test module per the post-PR-#45 convention
(`activation/tests.rs` → `tests/mod.rs` + `tests/keyboard.rs`,
`ipc/tests.rs` → `tests/mod.rs` + `tests/actions.rs`); both build on
`compositor/test_support.rs`'s `Harness`.

### The tests really can fail

Run against the unfixed code (fix stashed, tests kept) before being
accepted:

```
test compositor::activation::tests::keyboard::an_activation_takes_the_keyboard_back_from_a_clicked_taskbar ... FAILED
  assertion `left == right` failed: activating a window left the keyboard on the taskbar
test compositor::ipc::tests::actions::focusing_over_ipc_takes_the_keyboard_back_from_a_clicked_taskbar ... FAILED
  assertion `left == right` failed: focus action 0: the keyboard did not follow window focus
test compositor::ipc::tests::actions::a_layout_action_over_ipc_leaves_a_clicked_taskbars_keyboard_alone ... ok
test result: FAILED. 2 passed; 2 failed
```

Both failures land on the assertion that the seat's keyboard focus is the
window's `wl_surface` — the property itself, not a proxy. With the fix:
all three pass.

### Evidence

Dev VM (`ssh -p 2222 dev@localhost`), through the 9p mount at `/mnt/flexwm`,
`CARGO_TARGET_DIR=/var/cargo-target`, force-cleaned (`cargo clean -p flexwm`
first — a 9p build reporting `Finished` in under ~2s with no `Compiling`
line silently skips a real change). Final verification re-run on the
committed tree; see the PR report for the exact SHA.

```
cargo test -p flexwm                                  650 passed; 0 failed; 1 ignored
cargo nextest run --workspace                         744 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   0 warnings
cargo fmt --check -p flexwm                           clean
MODE=--headless scripts/smoke-test.sh                 12 `ok:` lines (private binary copy, per the shared-target-dir gotcha)
```

### Not benchmarked, and why

Nothing this touches is a per-frame or per-motion path. The activation clear
is one store on a token-redemption path; the IPC predicate is a
branch over an enum variant on an IPC-dispatch path, with one store when it
matches. No allocation, no new work inside any loop.
