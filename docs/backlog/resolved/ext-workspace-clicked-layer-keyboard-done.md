---
title: "`ext-workspace-v1` workspace activation moves window focus but leaves the keyboard on a clicked layer surface — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `ext-workspace-v1` workspace activation moves window focus but leaves the keyboard on a clicked layer surface — RESOLVED.

## The entry as filed

Found 2026-09-16, reviewing PR #53
(`resolved/activation-clicked-layer-keyboard-done.md`), which fixed the
identical bug in `xdg-activation-v1`'s `request_activation` and in the IPC
focus family, and left this caller untouched per ticket scope. The
independent reviewer verified it still calls `act` bare.

### The suspected gap

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

### Why this was suspected, not confirmed

Unlike the activation and IPC paths, no failing test demonstrates this
yet — nobody has put a real click, a real workspace switch, and a seat
keyboard-focus assertion together on this path. It is possible (though
unlikely) that workspace switching re-derives focus through a path that
does not consult `clicked_layer`, in which case there is no bug and this
entry should close as not-a-bug rather than collect a decorative clear.

## Resolution (2026-09-16, PR #54)

Confirmed as a real bug, not a not-a-bug: the two new tests below fail
against the unfixed code on the seat's actual keyboard-focus assertion,
which proves workspace switching re-derives focus through
`refresh_keyboard_focus` — the path that consults `clicked_layer` — and
nothing else.

### The fix

`commit_workspace_requests` now spends the click before switching,
mirroring `foreign_toplevel_management.rs:591` and PR #53's
`activation.rs` (including the comment style):

- a locked session is refused **before** the clear, the way PR #53's
  activation guard is ordered: pre-fix a locked switch was refused by
  `act`'s own gate with nothing disturbed; clearing first and refusing
  second would have spent the panel's click on a request that went
  nowhere. `act`'s gate stays as the backstop `shell.rs` describes it as.
  A stale index (nothing staged, or the workspace vanished before the
  commit) still returns before either, like an inert foreign-toplevel
  handle: ignored without touching anything.
- the already-active-workspace early return now spends the click too and
  runs the keyboard half (`refresh_keyboard_focus`), exactly
  `wlr_toplevel_activate`'s already-focused fast path. This adjudicates
  the entry's first open question: the same request over IPC
  (`FocusWorkspaceIndex`, PR #53) spends the click unconditionally, so
  this path agrees with it rather than differing by transport. The
  DoS guard the early return exists for is intact — still no `apply`,
  still no arrange/configure/render per repeat commit; the refresh is one
  serial and a focus compare Smithay no-ops when nothing moved.

### Tests

Three new tests in `ext_workspace/tests/keyboard.rs` (with the suite
split per the post-PR-#45 convention: `tests.rs` → `tests/mod.rs` +
`tests/keyboard.rs`, and the shared client grown a real `on_demand`
taskbar, a session-lock hold, and an unlock — the shapes
`foreign_toplevel_management/tests` already uses). Each runs a real mapped
`on_demand` layer surface from a second client, a real pointer click
through the input path, then a real `activate` + `commit` over the wire,
asserting on the seat's actual keyboard focus surface — never on a
hand-set field:

- `activating_another_workspace_takes_the_keyboard_back_from_a_clicked_taskbar`:
  two windows on two workspaces, taskbar clicked, switch workspaces;
  asserts window focus moved *and* the keyboard followed it.
- `activating_the_already_active_workspace_spends_the_click_too`: the
  early-return path; asserts the keyboard is on the window and the click
  spent, pinning the transport-agreement reasoning above.
- `a_workspace_activate_while_locked_spends_neither_focus_nor_the_taskbars_click`:
  lock, attempt the switch, assert refusal AND `clicked_layer` survival
  AND the keyboard back on the taskbar after unlock. Moving the clear
  above the `is_locked()` check fails it while leaving the other two
  green — it pins the refusal-before-clear ordering.

### The tests really can fail

Run against the unfixed code (fix stashed, tests kept) before being
accepted:

```
test compositor::ext_workspace::tests::keyboard::activating_the_already_active_workspace_spends_the_click_too ... FAILED
  assertion `left == right` failed: re-activating the current workspace left the keyboard on the taskbar
test compositor::ext_workspace::tests::keyboard::activating_another_workspace_takes_the_keyboard_back_from_a_clicked_taskbar ... FAILED
  assertion `left == right` failed: switching workspace left the keyboard on the taskbar
test compositor::ext_workspace::tests::keyboard::a_workspace_activate_while_locked_spends_neither_focus_nor_the_taskbars_click ... ok
test result: FAILED. 1 passed; 2 failed
```

Both failures land on the assertion that the seat's keyboard focus is the
window's `wl_surface` — the property itself, not a proxy. With the fix:
all three pass, and the whole `ext_workspace` suite passes
(`34 passed; 0 failed`).

### Evidence

Dev VM (`ssh -p 2222 dev@localhost`), through the 9p mount at `/mnt/flexwm`,
`CARGO_TARGET_DIR=/var/cargo-target`, force-cleaned (`cargo clean -p flexwm`
first — a 9p build reporting `Finished` in under ~2s with no `Compiling`
line silently skips a real change). Final verification re-run on the
committed tree; see the PR report for the exact SHA.

```
cargo test -p flexwm                                  654 passed; 0 failed; 1 ignored
cargo nextest run --workspace                         748 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   0 warnings
cargo fmt --check -p flexwm                           clean
MODE=--headless scripts/smoke-test.sh                 12 `ok:` lines (private binary copy, per the shared-target-dir gotcha)
```

### Not benchmarked, and why

Nothing this touches is a per-frame or per-motion path. The switch-path
clear is one store on a `commit` dispatch path; the already-active path
adds one `refresh_keyboard_focus` (a serial plus a focus compare Smithay
no-ops when nothing moved) where there was none — the same cost split PR
#50's fast path already accepted. No allocation, no new work inside any
loop.

### What this deliberately does not touch

- `ext-workspace-client-lookup-per-bind.md` (separate entry), toplevel
  screencopy, the rename.
- No `PROTOCOL_VERSION` bump and no `README.md` change: this is a bug fix
  with no new user-facing behavior — no new request, action, flag, binding
  or config. (Stated explicitly rather than silently skipped.)
