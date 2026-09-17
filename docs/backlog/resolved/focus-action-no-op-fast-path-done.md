---
title: "IPC focus actions run a full `apply` even when nothing moves — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# IPC focus actions run a full `apply` even when nothing moves — RESOLVED.

## The entry as filed

Found 2026-09-16, reviewing PR #54
(`resolved/ext-workspace-clicked-layer-keyboard-done.md`), deliberately
left out of that PR.

### The asymmetry

PR #54 gave `ext-workspace-v1`'s workspace activation an already-active
fast path: when the requested workspace is already active, it spends the
click and runs only the keyboard half (`refresh_keyboard_focus`) instead
of `act` — because otherwise a client repeating `activate` + `commit` as
fast as it can write to its socket drives a full `apply` (an arrange, a
configure per window, a render) per message, and the old code even said so
in its own comment.

The IPC focus family has no such fast path. `ipc.rs`'s
`Request::Action` handler converts and calls `self.act(...)`
unconditionally, so `focus-window-id` (or `focus-workspace-index`) for the
already-focused target runs the full `apply` every time. Same socket-speed
hazard, same cost, no guard.

### Why `low`, not higher

Exploiting it takes a client deliberately hammering focus actions at
socket speed — there is no evidence any real client does, and an agent
loop issuing one focus action per step pays one redundant arrange, which
is noise next to a screenshot encode. The ext-workspace guard exists
because that protocol *invites* repeat traffic (a bar re-asserting state);
nothing equivalent pressures the IPC path today. File now so the next
"focus feels slow under agent control" has somewhere to land; fix when a
measurement says so, not before.

### What it would take

Mirror the PR #54 split per focus variant: detect the no-op (target
already focused / already-active workspace) before `act` and run only the
keyboard half — with the `clicked_layer` clear PR #53 added kept on both
halves, and a test pinning each half (no-op spends the click without an
`apply`; real move still applies). Benchmark before/after on the IPC
dispatch path per the hot-path rule before claiming anything.

## Resolution (2026-09-17, PR #67)

Fixed exactly as filed, with one deliberate scope cut the ticket itself
anticipated.

### The fix

`Request::Action` in `ipc.rs` now checks `focus_action_is_noop` after the
session-lock gate and the PR #53 `clicked_layer` clear, and before `act`:
on a no-op it spends the click (already cleared above, so on both halves)
and runs only `refresh_keyboard_focus`, returning the same `ok()` reply
either way. The lock gate stays first, and `act`'s own gate stays as the
backstop `shell.rs` describes it as.

### Per-variant "already there" (also enumerated in the helper's docs)

Each is read off the state the core's own action would resolve against --
a wrong answer here silently drops a focus change, which is worse than a
redundant arrange:

- `FocusWindowId`: `State::focus` already names that window -- the same
  compare `wlr_toplevel_activate`'s fast path makes. An unknown id never
  equals it (`remove_window` clears `focus`, and ids are never reused), so
  it stays on the full path and keeps whatever handling `act` gives it
  today (currently still an `apply`).
- `FocusWorkspaceIndex`: the focused output's active workspace already is
  that index. Out of range can never equal `active` (`active < count`
  always), so it stays on the full path too. Read off the focused output
  rather than the single `OUTPUT_ID` `ext_workspace.rs` uses, because that
  is the list the core's own `reshape` resolves the index against (identical
  today; the read stays correct if outputs ever multiply).
- `FocusWorkspace`: the step clamps -- up from the first workspace, or
  down from the last (`active + 1 >= count`, underflow-free). The same
  lookup as the index case; the core then only re-sets the index it has
  and re-normalises an already-normalised tree, which is the reasoning PR
  #54's fast path states.
- `FocusColumn` / `FocusWindow`: deliberately left on the full path.
  No-op-ness there needs the focused column's position in its workspace
  (and the stack position within it), which `World` does not expose, so
  anything but the full path would be a guess. Resolving them would mean
  new core accessors for a socket-speed micro-opt; they keep today's
  behavior exactly, clear included. The helper's `_ => false` arm and the
  `relative_steps_stay_on_the_full_path` test change together if that ever
  lands.

The helper allocates nothing (an enum match plus, for the workspace
variants, two `Copy` reads off the core) -- no `arrange()`, no `Vec`.

### Tests

Five new tests in `ipc/tests/actions.rs`, all on the real-clicked-taskbar
fixture, all asserting on the seat's actual keyboard focus (never a
hand-set field), all using `needs_render` the way
`activating_the_window_that_is_already_focused_does_no_work_at_all` does
(set by `apply`'s `request_render` and by nothing else on this path):

- `already_focused_actions_spend_the_click_without_an_apply`: all three
  fast-path variants in a loop -- click, reset `needs_render`, request,
  assert `Ok`, focus unmoved, keyboard follows focus, click spent,
  `!needs_render`. The fail-first pin for the no-op half (below).
- `real_focus_moves_over_ipc_still_apply`: the contrast -- a real
  `FocusWindowId` move and a real `FocusWorkspace` step onto the trailing
  empty workspace (focus legitimately empties, keyboard follows it to
  nothing) both still `apply`. Passes with and without the fix, by design.
- `unknown_window_id_is_not_a_noop`: id 999 goes the full path
  (`needs_render` set), focus unmoved, click still spent, keyboard
  re-derived onto the kept focus.
- `out_of_range_workspace_index_is_not_a_noop`: index 99 goes the full
  path, workspace list bit-identical after.
- `relative_steps_stay_on_the_full_path`: `FocusColumn` left from the
  leftmost column and `FocusWindow` up from a one-window stack -- both
  core no-ops -- still `apply`, keyboard and click correct. Pins the scope
  cut above.

The PR #53 predicate coverage is unchanged (all five `Focus*` variants
still clear unconditionally) and its boundary test
(`a_layout_action_...leaves_a_clicked_taskbars_keyboard_alone`) passes
unmodified.

### The tests really can fail

Run against the unfixed code (fix stashed, tests kept) before being
accepted:

```
test compositor::ipc::tests::actions::already_focused_actions_spend_the_click_without_an_apply ... FAILED
  panicked at crates/flexwm/src/compositor/ipc/tests/actions.rs:506:9:
  no-op focus action 0 ran a full apply
test result: FAILED. 6 passed; 1 failed
```

The loop fails fast on variant 0 (`FocusWindowId`), which proves the
unconditional `act` ran the apply; the other 6 (the original two plus the
four edge/contrast tests, which pass either way by design) stay green.

### Benchmark (mandatory per the hot-path rule -- IPC dispatch is one)

Live socket flood on the dev VM (`ssh -p 2222 dev@localhost`), debug
build, `--headless` 800x600 with one `foot` window, one connection per
phase, every reply line counted. Two phases per action: 300 sequential
request->reply round trips (mean/median/p95 wall-clock per request) and
3000 pipelined requests in batches of 100 (sustained throughput). Batches,
not one burst: a single 3000-deep unread pipeline trips the compositor's
write-stall guard (connection dropped after ~10s of no progress -- its
documented self-defense, not dispatch cost), found while measuring.

Pre-fix binary: branch content identical to `main` at `17df74e` (verified
`Compiling` build, 9.12s -- not a 9p false-`Finished`). Post-fix binary:
same, after the fix (12.50s `Compiling`).

```
                          pre (17df74e)              post (this PR)
focus-window-id no-op:
  seq mean                205-224 us                 112-138 us
  seq median              186-206 us                  96-113 us
  flood                    48-50 us/req (~20k/s)       25-29 us/req (~34-41k/s)
focus-workspace-index no-op:
  seq mean                188-201 us                 139-155 us
  seq median              169-188 us                 121-155 us
  flood                    40-50 us/req (~20-25k/s)    22-27 us/req (~38-45k/s)
```

Raw runs (3x each, `NSEQ=300 NFLOOD=3000`, perl flood over one Unix
connection per phase):

pre:
```
focus-window-id: seq mean=204.6us median=186.0us p95=287.0us flood=48.5us/req (20616 req/s)
focus-workspace-index: seq mean=187.7us median=169.0us p95=268.0us flood=50.3us/req (19899 req/s)
focus-window-id: seq mean=223.5us median=206.0us p95=289.0us flood=48.3us/req (20702 req/s)
focus-workspace-index: seq mean=201.4us median=188.0us p95=252.0us flood=42.7us/req (23441 req/s)
focus-window-id: seq mean=211.5us median=200.0us p95=283.0us flood=50.0us/req (19985 req/s)
focus-workspace-index: seq mean=191.4us median=177.0us p95=246.0us flood=39.8us/req (25115 req/s)
```
post:
```
focus-window-id: seq mean=112.0us median=96.0us p95=209.0us flood=24.5us/req (40757 req/s)
focus-workspace-index: seq mean=154.7us median=155.0us p95=241.0us flood=22.3us/req (44767 req/s)
focus-window-id: seq mean=122.9us median=106.0us p95=230.0us flood=29.3us/req (34170 req/s)
focus-workspace-index: seq mean=148.2us median=142.0us p95=240.0us flood=26.2us/req (38130 req/s)
focus-window-id: seq mean=137.7us median=113.0us p95=219.0us flood=25.7us/req (38960 req/s)
focus-workspace-index: seq mean=138.6us median=121.0us p95=235.0us flood=26.6us/req (37584 req/s)
```

Sequential latency ~1.6-1.8x better, flood throughput ~1.7-1.9x (~20k to
~38k req/s). Debug profile throughout -- the numbers resolve the
difference clearly, so no release run was needed. The remaining per-request
cost is the unchanged socket/JSON/codec path plus one
`refresh_keyboard_focus` (a serial and a focus compare Smithay no-ops when
nothing moved).

### Bug bash (edges from the ticket, each checked)

- Unknown window id: not a no-op by construction (`focus` only ever names
  a live window), still goes through `act`; pinned by test.
- Out-of-range workspace index: `active < count` always, so never equal;
  pinned by test.
- Locked session: the lock gate precedes everything, unchanged and first;
  the fast path returns the same `ok()` the full path would have (lock
  state cannot change mid-handler -- single thread).
- Concurrent identical requests: serialized on the event loop; the first
  real move applies, the rest hit the fast path. Reasoned, not simulated
  (no harness for two racing IPC peers).
- `needs_render` already set on entry (e.g. by the click): the fast path
  only skips *setting* it, never clears -- a pending frame still renders.
- Popup grab live during a no-op: the fast path ends in the same
  `refresh_keyboard_focus` the full path's `apply` ends in, with identical
  inputs (nothing moved) -- same precedence outcome, including the
  interaction-serial recording guards.
- `self.focus` vs core focus drift: impossible between applies --
  `set_focus` runs in every `apply`, and `remove_window` clears `focus`
  directly before its own `apply` re-derives it.
- Relative-step scope cut: `FocusWorkspace` Up-from-0/Down-from-last skips
  a `normalize()` that is a fixpoint in steady state (every core mutation
  normalises, so empty non-active workspaces cannot exist between
  actions) -- the same reasoning PR #54 states for the index case.

### Evidence

Dev VM (`ssh -p 2222 dev@localhost`), through the 9p mount at `/mnt/flexwm`,
`CARGO_TARGET_DIR=/var/cargo-target`, force-checked (`cargo clean` not
needed -- both builds show real `Compiling flexwm` lines, 9.12s and
12.50s; a 9p skipped build reports `Finished` in under ~2s with no
`Compiling` line). Committed tree is byte-identical to the benchmarked
tree (`git status` clean after commit), so the evidence keys to the commit
SHA in the PR report.

```
cargo test -p flexwm                                  726 passed; 0 failed; 1 ignored
cargo nextest run --workspace                         825 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   0 warnings
cargo fmt --check -p flexwm                           clean
MODE=--headless scripts/smoke-test.sh                 all `ok:` lines, no `BUG`
```

### What this deliberately does not touch

- Focus semantics, the 64-connection cap, toplevel screencopy, the
  rename, `flexwm-core` (no new accessors -- the scope cut above).
- No `PROTOCOL_VERSION` bump and no `README.md` change: this is purely
  internal latency -- no new request, action, flag, binding or config.
  One observable difference exists in principle (fewer `configure` events
  a client counting them could observe on repeat no-op requests), but it
  is the same non-difference PR #54's fast path and
  `wlr_toplevel_activate`'s already accepted: a redundant configure
  skipped, never a missing one. (Stated explicitly rather than silently
  skipped, per the ticket.)

Found 2026-09-16, reviewing PR #54
(`resolved/ext-workspace-clicked-layer-keyboard-done.md`), deliberately
left out of that PR.

## The asymmetry

PR #54 gave `ext-workspace-v1`'s workspace activation an already-active
fast path: when the requested workspace is already active, it spends the
click and runs only the keyboard half (`refresh_keyboard_focus`) instead
of `act` — because otherwise a client repeating `activate` + `commit` as
fast as it can write to its socket drives a full `apply` (an arrange, a
configure per window, a render) per message, and the old code even said so
in its own comment.

The IPC focus family has no such fast path. `ipc.rs`'s
`Request::Action` handler converts and calls `self.act(...)`
unconditionally, so `focus-window-id` (or `focus-workspace-index`) for the
already-focused target runs the full `apply` every time. Same socket-speed
hazard, same cost, no guard.

## Why `low`, not higher

Exploiting it takes a client deliberately hammering focus actions at
socket speed — there is no evidence any real client does, and an agent
loop issuing one focus action per step pays one redundant arrange, which
is noise next to a screenshot encode. The ext-workspace guard exists
because that protocol *invites* repeat traffic (a bar re-asserting state);
nothing equivalent pressures the IPC path today. File now so the next
"focus feels slow under agent control" has somewhere to land; fix when a
measurement says so, not before.

## What it would take

Mirror the PR #54 split per focus variant: detect the no-op (target
already focused / already-active workspace) before `act` and run only the
keyboard half — with the `clicked_layer` clear PR #53 added kept on both
halves, and a test pinning each half (no-op spends the click without an
`apply`; real move still applies). Benchmark before/after on the IPC
dispatch path per the hot-path rule before claiming anything.
