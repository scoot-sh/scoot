---
title: "`zwp_tablet_manager_v2` (drawing-tablet input): tool motion + tip-click through the pointer paths, pads deferred upstream — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `zwp_tablet_manager_v2` — DONE (tool motion/buttons routed, pads deferred upstream)

Split out of the niche bundle
(`protocol-gaps-niche-done.md`, item 1) 2026-09-18 as its own entry
(`docs/backlog/input/tablet-v2.md`): advertising `TabletManagerState`
alone would have been dishonest — clients would bind it and get zero
tools, because flexwm had no tablet input path. This record closes that
entry with the path built.

## What landed

- **`zwp_tablet_manager_v2` (version 1) advertised.** `TabletManagerState`
  is Smithay's at the pinned rev (`0ff0098`,
  `src/wayland/tablet_manager/`), so this is the hold-alive shape: one
  field on `State` (`tablet_manager_state`), constructed in `State::new`.
  Version 1 is the maximum Smithay offers at the pinned rev
  (`MANAGER_VERSION: u32 = 1` in `src/wayland/tablet_manager/mod.rs`);
  the protocol XML itself is at version 2 (manager v2 adds `bustype` on
  tablets and pad dials — both on objects flexwm never mints, see pads
  below). Blanket `Dispatch`/`GlobalDispatch` in `dispatch.rs` cover the
  new objects with no per-interface work, the same way they cover the
  relative-pointer globals.
- **libinput tablet-event plumbing** (`tty/mod.rs::libinput_event`): the
  four `TabletTool*` arms the old `_ => {}` swallowed now translate to
  the backend-neutral `State::tablet_*` methods in the new
  `compositor/tablet.rs`, with positions mapped onto the output's
  *logical* size via `position_transformed` (the same mapping the
  absolute-pointer arm uses) and axis frames built from the
  `*_has_changed` getters (an unchanged axis is `None`, not a restated
  zero). `DeviceAdded`/`DeviceRemoved` register/forget tablets for
  `TabletTool`-capable devices, anvil's shape at the pinned rev
  (`anvil/src/input_handler.rs`, `on_device_added`/`on_device_removed`
  including the clear-tools-when-no-tablets-remain rule).
- **Tool focus and cursor through the existing paths, not a second focus
  system** (`tablet.rs`): proximity/motion run `pointer_move` (cursor
  follows, pointer focus/constraints/relative motion derived exactly as
  for the mouse), tip down/up runs `pointer_button` with the left button
  (a pen tap focuses, activates and clicks like a mouse click —
  interaction-serial record and popup-grab settle included), tool
  down/up/motion/axis/proximity go to whatever `surface_under` finds at
  the same location. `TabletSeatHandler::ToolFocus` stays `WlSurface`
  (it already was, for the cursor-shape bound), and `tablet_tool_image`
  lands in the same `Cursor` status a pointer image would. The tap is
  recorded once, as the pointer click it performs: tool down/up serials
  are deliberately not added to `interaction_serials`.
- **Barrel buttons are tool-only**: delivered as the tool's own button
  event with the exact button number; no pointer synthesis (no defined
  mapping exists, and inventing barrel-equals-right-click would be bespoke
  semantics no protocol asks for).

## Verify-first: what the pinned rev actually says

All re-verified in source at `0ff0098`, not from general Smithay
knowledge:

- The libinput backend produces all four tool event kinds
  (`src/backend/libinput/mod.rs`: `TabletToolAxis`, `TabletToolProximity`,
  `TabletToolTip`, `TabletToolButton` callbacks) — the plumbing has a
  real source on `--tty` hardware. No `TabletPad*` variant exists in
  `InputEvent` (`src/backend/input/mod.rs`), and the input tablet module
  says so outright ("Currently, pads are unsupported by this module").
- `TabletSeat::add_wp_tablet`/`add_wp_tool` are the wayland-exposing
  constructors (`src/wayland/tablet_manager/tablet.rs`,
  `tablet_tool.rs`); the plain `add_tablet`/`add_tool` announce nothing.
  Only proximity announces a tool (anvil announces on proximity too;
  motion/tip/button for an unknown tool stay tool-silent there as here).
- A focus-changing pointer motion sends `enter` with *no* `motion`
  (`src/input/pointer/mod.rs`, `PointerInternal::motion`'s `(focus, None)`
  arm) — found the hard way: the first version of the proximity test
  asserted a pointer motion on arrival and failed on real behavior, not
  on a bug. The test now asserts the enter on arrival and proves the
  cursor follows with a second move.
- The concrete libinput event structs carry *inherent* `tip_state` /
  `button_state` methods answering the input crate's enums, which shadow
  the Smithay trait methods in method-call syntax (a mismatch, not an
  ambiguity). The two call sites use fully-qualified trait syntax;
  Smithay's own libinput backend does the same internally.
- `get_tablet_seat` mints one seat object per call with no budget, the
  same standing exposure as `wl_seat.get_pointer`/`get_keyboard` — so no
  new bind-budget policy was invented for the tablet twin (stated, not
  overlooked).

## Demand

Still none observed, as the entry said: neither probed shell binds the
global, and no toolkit on the dev VM asks for it. What changed is the
honesty direction — the global is now backed by a real input path, so a
tablet-aware client (Krita, Xournal++) on `--tty` hardware gets tools
instead of silence, and every other client gets a pen that moves the
cursor and clicks.

## Evidence

Harness suite `compositor/tablet/tests.rs` (8 tests): real
`wayland-client` connections through a real `State`, synthetic tool
events through the same `State::tablet_*` methods libinput calls, wire
assertions on both the tool and the pointer halves.

Fail-first (dev VM, branch at the implementation commit, advertisement
neutered Mac-side to `Option<TabletManagerState> = None` — compiles,
advertises nothing): 6 fail (all six wire tests), 2 pass (the two unit
pins over descriptor contents and the pointer button-state ints, which
pin constants rather than behavior and are unaffected by design).

```text
test compositor::tablet::tests::pen_proximity_announces_tablet_and_tool ... FAILED
test compositor::tablet::tests::pen_proximity_out_parks_cursor_and_ends_tool_stream ... FAILED
test compositor::tablet::tests::pen_barrel_button_is_tool_only ... FAILED
test compositor::tablet::tests::pen_motion_reports_movement_without_stale_pressure ... FAILED
test compositor::tablet::tests::pen_tip_clicks_and_focuses_like_a_mouse ... FAILED
test compositor::tablet::tests::tablet_manager_is_advertised ... FAILED
test result: FAILED. 2 passed; 6 failed; 0 ignored; 0 measured; 944 filtered out
```

Post-fix, same command: 8 pass (`cargo test -p flexwm -- tablet`).

Full standard set on the dev VM at the final tree: `cargo test -p
flexwm` 951 passed / 0 failed / 1 ignored; `cargo nextest run
--workspace` 1056 passed / 1 skipped; `cargo clippy -p flexwm
--all-targets -- -D warnings` clean; `cargo fmt --check -p flexwm`
clean; `scripts/smoke-test.sh` 17 ok.

Live advertisement (dev VM, `--headless` at the implementation commit):
`wayland-info` lists `zwp_tablet_manager_v2`, version 1.

No hot-path benchmark: no existing hot path changed. `move_absolute`,
`pointer_button` and the libinput keyboard/pointer arms are byte-identical;
the new work is four match arms that only run on tablet-device events
plus per-event seat/tool lookups beside the hit test the pointer half
already pays (an `Arc` clone and one map lookup; `add_wp_*` only for a
never-seen tool/tablet, never per event). The axis frame is a stack
struct; nothing allocates per event beyond what `pointer_move` does.

## What this deliberately leaves open (and why)

- **No tablet-tool hardware exists on the dev VM**, so the libinput arms
  never ran against a real device — stated plainly. `libinput
  list-devices` shows four devices (two keyboards, `gpio-keys`, and a
  "QEMU QEMU USB Tablet" that reports only the `pointer` capability, so
  libinput delivers it as absolute-pointer motion, never tool events).
  The seat-level synthetic tests drive the exact methods the arms call;
  the arm bodies themselves (transform + forward, anvil's shape) are
  review-verified, not live-verified. Real tool types (pen vs eraser vs
  airbrush behavior differences) are untested for the same reason. The
  scenario that revisits this is the entry's own: a Wacom-style tablet
  on `--tty` hardware plus Krita/Xournal++. A uinput-built virtual
  tablet was considered and declined: it needs root uinput programming
  plus seat wiring for what the synthetic tests already prove at the
  seat level.
- **Pads, strips, rings and dials: deferred upstream, not omitted.**
  Smithay carries no pad objects at the pinned rev (no `TabletPad*`
  input events, "pads are unsupported" in the module doc), so there is
  nothing to drive — the client-visible `pad_added` can never fire.
  Recorded here and in the README, not silently dropped.
- **Tool down/up serials are not interaction evidence** (the tap's
  pointer-click serial is). A tablet-aware client mints activation from
  the click like every other client; no second serial namespace exists
  to guess from.
- **Session-lock behavior rides the two reused paths** (`surface_under`
  answers the lock surface; `focus_under_pointer` refuses to move focus
  behind it) and has no tablet-specific test — the lock suites already
  pin both paths, and a third derivation could only disagree with them.

## Bookkeeping

- Open entry `docs/backlog/input/tablet-v2.md` deleted in-PR (it becomes
  this file).
- `protocol-gaps-niche-done.md`'s sub-item 1 link repointed here (it
  pointed at the now-deleted open path).
- README: protocol-list paragraph plus a "Drawing tablets" section.
- ROADMAP "Recently shipped" bullet.
