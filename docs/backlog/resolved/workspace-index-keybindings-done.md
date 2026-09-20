---
title: "Workspace shortcuts: no default bind for a numbered workspace, and no move-to-index action at all — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Workspace shortcuts: no default bind for a numbered workspace, and no move-to-index action at all — RESOLVED

## What it said

Two gaps that look like one. Gap 1 (keybinding only): `focus-workspace-index N`
existed in `scoot-core`, over IPC and in the config grammar, but had no default
bind. Gap 2 (missing action): `move-window-to-workspace` took only `up|down` —
no index form anywhere, so a window could only be stepped one workspace at a
time, including by agents (stepping is not idempotent the way an absolute
target is). Shape: mirror `FocusWorkspaceIndex` — core action, `scoot-ipc`
variant and conversion, config-file grammar, default binds `Super+1`..`9` and
`Super+Shift+1`..`9`. Additive wire format, no version bump. Naming workspaces
explicitly out of scope.

## Resolution

Shipped as filed, both halves. The ticket's "decide what a bind for a
workspace that doesn't exist yet does" closed without a behavior debate, per
the coordinator's mirror-what-exists decision — and the code made it a
non-decision twice over:

- **Out-of-range semantics as found:** `Output::focus_workspace_index`
  (`crates/scoot-core/src/world/tree.rs`) ignores an index past the end —
  no create, no clamp — with a doc comment saying why (a stale list must
  not silently activate the last workspace). Gap 1 changes no behavior, so
  the new binds inherit that. The new
  `Output::move_focused_window_to_workspace_index` mirrors it exactly: an
  unknown index leaves the window where it is, moving to the already-active
  index and moving with no window focused are early-return no-ops — the
  same early returns the relative move makes at the tree's edge and on an
  empty workspace. Documented in one sentence in `docs/configuration.md`
  ("does nothing: it neither creates a workspace nor falls back to the
  last one") and on both new enum variants.
- **No conflicts:** nothing was bound on `Super`+digits or
  `Super+Shift`+digits (defaults were `h`/`j`/`k`/`l`/`r`/`q`/`Return`/`e`;
  `--tty` VT switches are `Ctrl+Alt+F1`..`F12`). Verified, not assumed —
  by reading the table and by the new bind test failing pre-change.
- **No version bump:** the new `Action` variant is client→server only (the
  server never sends actions), so old clients decode exactly as before —
  the same additive shape the protocol-bundle item used.
- **No fast-path change:** `focus_action_is_noop` covers focus actions
  only; the new action is arrangement, so it stays on the full `act` path
  like the relative move — and stays out of the focus-family
  click-spending match, pinned by a boundary test mirroring the
  layout-action one.

What landed, layer by layer: `Action::MoveWindowToWorkspaceIndex(usize)` in
`scoot-core` (+ `handle_action` arm + `Output` method), the `scoot-ipc`
variant + conversion, the `scootctl::action` grammar arm (which the config
`[binds]` parser and `[autostart]` entries reuse — no parallel grammar),
the shared `ACTIONS_HELP` block both `--help` outputs embed, 18 default
binds (`Super+1`..`9` → index 0–8 focus, `Shift` added → carry + follow),
and docs (`README.md` Keys rows, `configuration.md` grammar + binds table
+ the nonexistent-index sentence, `ipc.md` grammar + the shared 0-based /
out-of-range sentence — where `move-*` already covered the keyboard
boundary, so nothing changed there).

## Evidence

- Fail-first: the new `scootctl` parse test failed pre-arm with
  `Err(Unknown("move-window-to-workspace-index"))`; the new default-binds
  test failed pre-binds (`DIGITS` unbound); every new behavior test was
  written against the new API.
- `cargo nextest run --workspace`: 1182 passed, 4 skipped (dev VM, Linux —
  includes 5 new core tests, the fuzz generator's new arm, the wire
  round-trip both directions, the `scootctl` parse test, the config
  digit-combo test, the binds test, and 2 new compositor IPC tests); 144
  passed Mac-side. `cargo clippy --workspace --all-targets -- -D warnings`
  clean both sides, `cargo fmt --check` clean both sides.
- `scripts/smoke-test.sh` green on the dev VM (all `ok` sections, exit 0).
- Live `--headless` proof on the dev VM (`/tmp/wsidx-proof.sh`, exit 0):
  moves on an empty session answer `Ok` without crashing; two `foot`
  windows mapped, `action move-window-to-workspace-index 1` carried the
  focused window (other window `visible: false`, focus followed); injected
  `super+1` refocused workspace 0; injected `super+shift+2` carried the
  window back with focus following; `super+9` / `super+shift+9` left the
  window list byte-identical; session answered `version` after.
- Hot path: `match_key` is once-per-keypress over a now-36-entry table, no
  allocation in the path (`iter().find()`). Measured release on the dev VM
  (temporary bench, removed after): full-table miss 32.7ns/call,
  first-entry hit 3.9ns — ~15ns over the old 18-entry worst case, noise
  against a 16ms frame.

Left out, with why: named workspaces (ticket's explicit exclusion);
changing the existing relative binds (out of scope); a no-op fast path
for move-to-current-index (deliberate scope cut, same as the relative
move — detection would need core accessors for a socket-speed micro-opt).
