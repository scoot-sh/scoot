---
title: "Quickshell's window list needs `wlr-foreign-toplevel-management-v1`; it ignores the `ext-` list flexwm now advertises — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Quickshell's window list needs `wlr-foreign-toplevel-management-v1`; it ignores the `ext-` list flexwm now advertises — RESOLVED.

## The entry as filed

Found 2026-09-16 while implementing `ext-foreign-toplevel-list-v1`
(`resolved/foreign-toplevel-list-done.md`), by probing the client the
original entry was filed for rather than assuming it would follow the
standard.

## What was measured

Stock `quickshell` 0.3.1 (the exact build both the DMS and Noctalia probes
used, from the dev VM's nix store), a minimal `ShellRoot` whose only job is
to read `Quickshell.Wayland.ToplevelManager`, against `flexwm --headless`
with one real `foot` window open:

```
--- flexwm msg windows ---
[{"id":1,"app_id":"foot","title":"dev@flexwm-vm: ~"}]
--- quickshell ---
DEBUG qml: QS: initial count = 0
```

`WAYLAND_DEBUG=1` on the same run says why. flexwm offers the global and
quickshell simply does not take it:

```
wl_registry#2.global(7, "ext_foreign_toplevel_list_v1", 1)
```

— offered five times across four registries (main twice, plus three mesa
ones), bound zero times; `grep -n "bind.*foreign" /tmp/qs-wire.log` is empty. Its
binary carries a complete *wlr* client instead
(`zwlr_foreign_toplevel_manager_v1{,_listener,handle_toplevel,handle_finished}`
and `zwlr_foreign_toplevel_handle_v1handle_state`, …); the
`ext_foreign_toplevel_*` symbols in it are the scanner's interface tables
plus `ext_foreign_toplevel_image_capture_source_manager_v1`, which is a
different protocol.

So `ToplevelManager` — 29 use sites in DMS 1.5.3's QML, and Noctalia's
equivalent — is a `wlr-foreign-toplevel-management-unstable-v1` client. The
`ext-` list does not feed it, and the "Windows"/running-apps section of
both launchers stays empty until flexwm speaks the wlr protocol too.

## What this does and does not change

It does not undo the choice of protocol. `CLAUDE.md`'s rule (implement the
compositor-agnostic successor where one exists) still points at
`ext-foreign-toplevel-list-v1`, the pinned Smithay rev implements that and
nothing else, and it is what a standards-following client gets. The wlr
protocol is now the *compatibility* question it always was, with one fewer
unknown: the two shells this repo cares about need it.

## Why it is not a small follow-up

- **Nothing in Smithay implements it** at the pinned rev (grep: no
  `foreign_toplevel_management` anywhere), so this is a hand-rolled global,
  object lifecycle and event batching against `wayland-server` — the shape
  `ext_workspace.rs` already is, and about that size.
- **It is a control protocol, not an enumeration one.** Beyond
  title/app_id/output it carries `state` (maximized, minimized, activated,
  fullscreen) and accepts `activate`, `close`, `set_maximized`,
  `set_minimized`, `set_fullscreen`, `set_rectangle`. flexwm's core has
  `activate` and `close` and **no concept at all** of maximized, minimized
  or fullscreen — so the real work is deciding what those mean in a
  scrolling-column layout (or advertising them as unsupported and having a
  taskbar's minimise button silently do nothing), not the wire format.
- `set_rectangle` is a task-switcher animation hint; harmless to ignore.

A first cut could be enumeration + `activate` + `close` only, with the
state bits it can honestly answer (`activated`) and nothing else — which
would light up both shells' window lists and their click-to-focus, and
leave minimise/maximise for whenever the core grows them.

## Worth checking first, cheaply

Whether a newer quickshell binds `ext-foreign-toplevel-list-v1` when the
wlr global is absent. 0.3.1 does not, but this protocol is young and
quickshell moves fast; a five-minute re-probe of the current release with
the same minimal QML (kept in this entry's evidence, and trivially
rebuilt) answers it, and a yes would make this entry a compatibility
nicety instead of the thing standing between flexwm and two working
shells.

## The cheap re-check, first

Answered before any code was written, and the answer was "nothing has
changed": the dev VM's nix store holds exactly one quickshell, and it is
the **same build** the original measurement used —
`/nix/store/hnw9kk48z8jqp0pqha5gwnpyawpcxq34-quickshell-0.3.1`,
`sha256:061rh2p8frgxkz450g5x55axab3kjk8vg6530m4mzqa2jzddmivg`. There is no
newer release in the store to re-probe, and a store path is its own content
hash, so re-running the QML against a bit-identical binary could only
reproduce the recorded result.

What its binary carries, as a second check on the original finding
(`nm -a` on `bin/.quickshell-wrapped`, the real ELF behind the wrapper):

```
zwlr_foreign_toplevel_manager_v1   32 symbols
ext_foreign_toplevel_list_v1        4 symbols
```

— and the four are only `_events`, `_interface`, `_requests`, `_types`,
i.e. the scanner's interface tables with no client code hanging off them,
while the wlr side has the complete client (`init`, `init_listener`,
`handle_*` callbacks, and the request wrappers `activate`, `close`,
`set_maximized`, `set_minimized`, `set_fullscreen`, `set_rectangle`, plus
the version-3 `handle_parent`). So: still a wlr client, still not an `ext-`
one. Implementation proceeded.

## Resolution (2026-09-16, PR #50)

New module `compositor/foreign_toplevel_management.rs` (plus its three test
files), implementing the first cut this entry scoped: enumeration,
`activate`, `close`, and `activated` as the one honest state bit. Wired to
the same window-lifecycle choke points `foreign_toplevel.rs` already uses —
`shell.rs`'s `add_window`, `remove_window` and `refresh_window` — plus
`set_focus` for the state bit and `handlers.rs`'s `output_bound` hook for
`output_enter`. Nothing new tracks windows; both protocols read the window
model flexwm already has, which is what makes the two lists one list
described twice.

Smithay still has nothing for this protocol at the pinned rev, so the
global, the object lifecycle and the event batching are hand-rolled against
`wayland-protocols-wlr`'s generated server bindings through
`Dispatch2`/`GlobalDispatch2` — the shape `output_management.rs` (PR #49)
and `gamma_control.rs` already are.

### The decisions worth recording

**1. Both protocols, not a switch.** `CLAUDE.md`'s rule still points at the
`ext-` successor and it is what a standards-following client gets;
`foreign_toplevel.rs` is unchanged. This is the compatibility half, and the
justification for the exception is measurement, not preference — the two
shells this repo cares about bind the older protocol and nothing else.

**2. State: `activated` and nothing else.** flexwm's core has no concept of
maximized, minimized or fullscreen, so those bits are never sent and
`set_maximized`/`set_minimized`/`set_fullscreen` (and the `unset_` halves)
are accepted and ignored. A taskbar's minimise button does nothing, which is
the truth; inventing a meaning for those in a scrolling-column layout is
layout design and stays a separate question. The `activated` bit is
reconciled from `State::focus` in `refresh_wlr_activation` — the same field
`xdg_toplevel`'s own `activated` and the focus ring read — rather than
diffed from the two ids that moved, because `remove_window` writes that
field directly without going through `set_focus`.

**3. `set_rectangle` is ignored, including an invalid one.** wlroots posts
`invalid_rectangle` for a negative size because it *uses* the rectangle for
its minimise animation. flexwm reads nothing from it, so disconnecting a
shell over a number nothing looks at would be a worse answer than ignoring
it. Pinned by `the_state_requests_are_accepted_and_do_nothing`, which sends
`(0, 0, -1, -1)` and asserts the client is still being served afterwards.

**4. Version 3 is advertised and `parent` is never sent.** Version 3's one
addition is that event, and flexwm's layout has no parent/child relation —
every `xdg_toplevel` is an independent column entry, dialogs included. A
version 1 or 2 client would see exactly the same picture, so capping the
version would remove a client's ability to ask without adding a fact.

**5. `activate` is exactly what clicking the window is, and that took two
tries.** Going through `State::act` unconditionally would let a client drive
a full `apply` (arrange, a configure per window, a render) as fast as it can
write — the hazard `ext_workspace.rs` already guards against for workspace
`activate` — so an already-focused window skips `act` and runs only the
keyboard half.

The first version of that guard called `refresh_keyboard_focus` and stopped
there, reasoning from `set_focus`'s doc ("its unconditional
`refresh_keyboard_focus` is the only thing that takes the keyboard back off"
a clicked `on_demand` layer surface). **That was not enough, and review
caught it.** The refresh is only half of what `input.rs`'s
`focus_under_pointer` does; the other half is the line before it,
`self.clicked_layer = None`. `layer_shell.rs`'s `layer_keyboard_focus` reads
that field and hands the keyboard straight back to a still-mapped `on_demand`
surface — so without clearing it the refresh re-derives the *taskbar* and
nothing moves, which is the exact two-path disagreement the guard existed to
prevent, relocated rather than fixed.

So the click is now spent before either branch, mirroring `input.rs`
statement for statement, and the branch split is purely about cost. The
session-lock check moved ahead of both, since the fast path never reaches
`act`'s own gate and a refused request must not spend the taskbar's click
either.

The test that was supposed to pin this could not: it hand-set
`keyboard_on_layer` with no real layer surface in the fixture, so
`layer_keyboard_focus` returned `None` whatever the guard did and the
assertion passed either way. It is now built on a real mapped `on_demand`
layer surface with a real `wl_shm` buffer, clicked through the real pointer
(`taskbar_holding_the_keyboard`), and asserts on the seat's actual keyboard
focus surface. Both the already-focused and the not-focused branch are
covered, and both were confirmed to **fail** against the unfixed code before
being accepted — see the evidence below.

**6. `stop` leaves the handles working.** The protocol's teardown is stop,
wait for `finished`, then destroy the handles — which a client cannot do
safely if the compositor has stopped reporting them. So managers and handles
are stored separately (wlroots' own split) rather than handles being owned
by the manager that created them.

### What this does not do

- **No `output_leave`, ever.** flexwm has one output, a window is on it for
  its whole life, and a workspace switch does not move it. wlroots-based
  compositors keep the output assignment across a workspace switch for the
  same reason.
- **No cap on how many times one client may bind the global.** Each bind
  creates one handle object per window, and every window change and every
  focus change now walks them. Filed with the three other globals of this
  shape in `protocols/ext-workspace-object-binding-cap.md`, which this entry
  adds to rather than duplicating.

## Evidence

Everything below was captured at **`1ade117`**, the review-fix commit —
re-run from scratch rather than carried over from the first round, because
that commit changed what `activate` does and the earlier transcripts were
therefore stale for it. Dev VM (`ssh -p 2222 dev@localhost`), debug build
through the 9p mount at `/mnt/flexwm`,
`CARGO_TARGET_DIR=/var/cargo-target`, force-cleaned (`cargo clean -p flexwm
&& cargo build -p flexwm`) first, because a build through that mount can
otherwise report `Finished` in under two seconds without recompiling a real
change.

The dev VM's target directory was shared with a second implementer
throughout, and a build from *their* tree replaced
`/var/cargo-target/debug/flexwm` under someone else between a build and a
probe during this PR's review round — not a queueing delay, an actual wrong
binary. So the smoke test and every live probe here were pointed at a
**copy** taken in the same command as the clean build
(`/var/tmp/flexwm-r2`, since deleted), never at the shared path. Each probe
is also self-verifying on that point: a binary without this PR does not
offer the global the logs below show being bound.

### The checks

```
cargo test -p flexwm            TEST EXIT=0     622 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  716 tests run: 716 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm                           FMT EXIT=0
MODE=--headless scripts/smoke-test.sh                 SMOKE EXIT=0 (12 `ok:` lines,
                                                      screenshot /tmp/flexwm-smoke.png,
                                                      FLEXWM=/var/tmp/flexwm-r2)
```

69 of those tests are this protocol's own and the `ext-` one it must agree
with (`cargo test -p flexwm foreign_toplevel`: `69 passed; 0 failed`, 554
filtered out).

### The keyboard tests really can fail

The review's blocking finding was that the *previous* version of this test
could not have caught the bug it was named for. So the replacements were run
against the unfixed code before being kept: with the `self.clicked_layer =
None` line in `wlr_toplevel_activate` commented out and nothing else
changed,

```
cargo test -p flexwm foreign_toplevel_management::tests::requests
EXIT=101
  activating_the_focused_window_takes_the_keyboard_back_from_the_taskbar ... FAILED
    panicked at .../tests/requests.rs:138
  activating_a_different_window_takes_the_keyboard_back_too ... FAILED
    panicked at .../tests/requests.rs:171
test result: FAILED. 11 passed; 2 failed
```

Both failures land on the assertion that the seat's keyboard focus is the
window's `wl_surface` — i.e. on the property itself, not on a proxy for it.
With the line restored: `45 passed; 0 failed`.

### Live, against the real quickshell

Three runs, scripts kept on the dev VM so this can be re-run without
rebuilding them. `/var/tmp/qs-probe.sh` is PR #47's original probe,
untouched; `/var/tmp/qs-control-probe.sh` and
`/var/tmp/qs-activate-probe.sh` are this PR's. The `-1ade117.sh` copy of
each is the one that produced the transcripts here — identical but for the
binary path described above. Raw output at `/tmp/p1.log`, `/tmp/p2.log`,
`/tmp/p3.log`, with the full `WAYLAND_DEBUG=1` wire logs at
`/tmp/qsctl-wire.log` and `/tmp/qsact-wire.log`. (The `-699d940.sh` copies
from the first round are still there too; they point at a binary that has
been deleted, so re-point them before use.)

**1. The original probe, unchanged, now passes.** Same script that measured
`count = 0` when this entry was filed; one real `foot` window:

```
--- flexwm msg windows ---
[{"id":1,"app_id":"foot","title":"dev@flexwm-vm: ~"}]
--- quickshell ---
wl_registry#2.bind(8, "zwlr_foreign_toplevel_manager_v1", 3, new id [unknown]#29)
zwlr_foreign_toplevel_manager_v1#29.toplevel(new id zwlr_foreign_toplevel_handle_v1#4278190080)
zwlr_foreign_toplevel_handle_v1#4278190080.title("dev@flexwm-vm: ~")
zwlr_foreign_toplevel_handle_v1#4278190080.app_id("foot")
zwlr_foreign_toplevel_handle_v1#4278190080.output_enter(wl_output#18)
zwlr_foreign_toplevel_handle_v1#4278190080.state(array[4])
zwlr_foreign_toplevel_handle_v1#4278190080.done()
DEBUG qml: QS: changed count = 1
DEBUG qml: QS:   toplevel appId=foot title=dev@flexwm-vm: ~
```

(The `count = 0` still printed just before it is `Component.onCompleted`
firing before the bind's round trip — quickshell's own timing, not an empty
list.)

**2. `close`, and the `activated` bit, with two windows.** Two `foot`s
titled `alpha` and `beta`; QML calls `close()` on `alpha`:

```
=== windows before quickshell ===
[{"id":1,"title":"alpha","focused":false},{"id":2,"title":"beta","focused":true}]
QS:   [0] appId=foot title=alpha activated=false
QS:   [1] appId=foot title=beta  activated=true
QS: CLOSE -> alpha
QS: after-close count = 1
QS:   [0] appId=foot title=beta activated=true
=== windows 11s in (after CLOSE) ===
[{"id":2,"title":"beta","focused":true}]
```

Both `activated` bits match `flexwm msg windows`' `focused`, and the close
reached the right one of the two.

**3. `activate` — and a real client-side finding on the way.** The same run
above called `activate()` from a bare `ShellRoot` and *nothing happened*.
The wire says why, and it is not flexwm: quickshell never sent the request.

```
=== requests quickshell sent on foreign-toplevel objects ===
 -> zwlr_foreign_toplevel_handle_v1#4278190080.close()
 -> zwlr_foreign_toplevel_handle_v1#4278190080.destroy()
```

`activate` takes a `wl_seat`, and quickshell sources it from Qt's last input
device — which a shell with no window and no input event does not have. So
the probe was rebuilt around a real `PanelWindow` (a layer surface, exactly
what DMS and Noctalia are) with a `MouseArea` that activates the first
toplevel, and the click was injected through flexwm's own IPC:

```
=== windows before the click ===
[{"id":1,"title":"alpha","focused":false},{"id":2,"title":"beta","focused":true}]
=== clicking the quickshell panel at (600, 20) ===
{ "type": "ok", "locked": false }
=== windows after the click ===
[{"id":1,"title":"alpha","focused":true},{"id":2,"title":"beta","focused":false}]
QS: CLICK -> activate alpha
QS: after-click count = 2
QS:   [0] appId=foot title=alpha activated=true
QS:   [1] appId=foot title=beta  activated=false
=== requests quickshell sent on foreign-toplevel objects ===
 -> zwlr_foreign_toplevel_handle_v1#4278190080.activate(wl_seat#14)
```

That is the whole gesture end to end: a real shell panel, a real click, the
`activate` request on the wire, flexwm's focus moving, and quickshell's own
`activated` bits following it back. Worth knowing for anyone re-running
this: a headless quickshell probe **cannot** test `activate`, and a null
result from one means only that Qt had no seat.

### Not benchmarked, and why

Nothing this touches is a per-frame or per-motion path. The work lands on
window open, window close, a real title/app-id change, a `wl_output` bind
and a **focus change** — the last being the only one a user drives
repeatedly, and it is the branch of `shell.rs`'s `set_focus` that already
only runs when the focused window actually changed. What was added inside it
is a walk of the same window set that branch already walks, plus a `bool`
compare per window, sending nothing for the windows whose bit did not move
(every window but at most two). The generated `state(Vec<u8>)` signature
forces one four-byte allocation per handle whose bit really flipped; the
empty array allocates nothing. Bind-time announcement is O(windows) and
window open is O(bound managers), neither on a hot path.
