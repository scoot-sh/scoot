---
title: "Agent interface: `query`, `invoke`, `layout` and `subscribe` on the bar's socket"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M4"
resolved: "2026-09-30"
---

# Agent interface — RESOLVED

Resolved 2026-09-30 (draft PR, third of the M4 stack, on the `exec` / `push` /
`button` PR). What landed, the decisions, the ratchet and the evidence are
below the original entry, which is kept as filed. **The resource ratchet for it
is measured on the dev VM only (below); the run on the real hardware is still
to do.**

Filed 2026-09-29. Serves **computer use** (the reason to make the bar
machine-readable) and daily-drive (scripts). The bar's socket is separate from
scoot's IPC (`README.md`), so these live in `scootbar msg`.

An agent driving a desktop should not have to OCR a screenshot to read the
battery, or hunt pixels to press mute.

## Requests

- **`query [ID]`**: each module's state as JSON: id, output, text, class,
  value where it has one (percent, muted, SSID, active workspace). One request,
  bounded reply. Defined in [config-cli-and-reload](config-cli-and-reload-done.md);
  this entry adds the fidelity guarantees below.
- **`invoke ID ACTION [ARG]`**: run a module's action exactly as its click or
  scroll binding would (`toggle-mute`, `raise 5`), with no pointer involved. It
  goes through the same code path as `on_input`, so what an agent does and what
  a user does cannot diverge. An unknown module or action is a named error.
- **`layout`**: each module's rectangle in output coordinates and the output's
  origin, so an agent that prefers to click can aim `scoot msg` pointer
  injection at real coordinates. Reflects the last drawn layout, not a guess.
- **`subscribe [KIND...]`**: dedicate a connection to change events (`module`
  updates, `output` added or removed), following scoot's subscribe rules
  (`docs/ipc.md#events`): named kinds only, a subscribed connection serves no
  further requests, a subscriber that stops reading is disconnected, never
  buffered. Coalesced to the frame rate.

## Fidelity rule

`query` and `layout` must agree with what a screenshot shows, or the bar is
lying to an agent. Pin it with a test that compares `query` text and `layout`
rects against a headless-scoot screenshot.

## Bounds

Reply size caps, subscriber count cap, and zero cost with no subscriber (one
branch), as in [robustness-and-limits](../robustness-and-limits.md).

## Done when

An agent reads every module's value and presses a button through the socket,
and the fidelity test passes on two outputs at two scales.

## What landed

[`docs/scootbar/cli.md#the-agent-interface`](../../cli.md#the-agent-interface)
is the reference.

- **`query [ID]`** now takes a module id (an id that is not placed is an error
  naming the placed ones) and each entry carries a `tooltip` (absent while
  empty) and a `value` where the module has one: the workspaces module's
  `{"active": 2, "workspaces": [1, 2, 3]}` for that output. It is written from
  the very state the screen is drawn from (`daemon/agent.rs`), so it cannot
  disagree with a screenshot. Bounded to 512 KiB, which the worst entry times
  every module times eight outputs stays far inside (asserted by arithmetic).
- **`layout`**: per output, its name, origin, scale and `bar` rectangle (`null`
  while hidden or not configured), and each module's rectangle, all in the
  compositor's global logical pixels **as last drawn**: the scene's own spans (the
  layout the last committed frame used) converted at the scale that frame was
  drawn at, rounded outward (`Scale::logical_floor` / `logical_ceil`), so a
  pointer on any drawn pixel of a module is inside its rectangle and a pointer
  one pixel outside is not.
- **`invoke ID ACTION [N] [--output NAME]`** goes through `action::perform`, the
  one path a pointer press ends in. `ACTION` is a module's own action or a
  trigger (`click` ... `scroll-down`), which runs the configured binding. Every
  wrong ask (not placed, no such action, no such binding, a number where none is
  taken, a scroll's steps outside 1 to 32, an output that does not show it) is a
  named error that runs nothing.
- **`subscribe [module] [output]`** (`daemon/events.rs`, `control/conn.rs`): a
  `subscribed` reply and then one JSON line per event. Events are coalesced to
  the frame rate (a batch at most every 16 ms, the loop's poll timeout holding
  one back and existing only while one is held), a reload tells every module
  again, a subscribed connection serves no further requests (one error line
  however many arrive, including a request pipelined behind the `subscribe`),
  at most 4 subscribers, and a subscriber that cannot take a batch in one
  nonblocking write is dropped, never buffered. With no subscriber, one branch a
  loop turn and nothing recorded.

## Decisions

1. **The subscriber cap is 4**, not scoot's value. A bar has one socket and the
   readers it has are an agent and a script or two; each held connection is a
   file descriptor in a loop that polls a few dozen. **A maintainer call** if a
   use needs more.
2. **A subscriber is dropped on a short write, whole events only.** The
   alternative, a per-subscriber queue with a cap, is the buffering the entry
   rules out, and a half-written event is a corrupt line. An event larger than
   what the socket takes at once (a batch of many modules) ends the subscriber;
   the socket buffer is the kernel's (about 200 KB), and a batch is a few
   hundred bytes a module.
3. **`layout` lists modules with something to show only.** A module whose view
   is empty takes no space and has no rectangle, as on screen; a hidden bar has
   none at all. An agent that wants "is it placed" asks `query`.
4. **An exec binding runs once for `invoke ID scroll-up N`**, as `N` notches
   coalesced into one frame do on the pointer path (the steps go to a module's
   own action, which moves that many places; an `exec` runs once). Same code,
   same answer, so a test that drives either agrees.
5. **`invoke` and `query` allocate per request** (the list of outputs that show
   the module, the workspaces `value`'s JSON). They are one request each, at an
   agent's rate, not per pointer event or frame; the pointer and the event path
   (`pump_events` building into reused buffers) are the ones held to zero.
6. **The `value` is a module-defined JSON value**, not a typed field: the
   volume, battery and network modules will each put their own (percent and
   muted, a percentage, an SSID) there without a protocol change.

## The ratchet

The same bench, dev VM and settings as the entries before it:
`bench.py run --scope clock-workspaces --compositors scoot --rounds 3
--settle-secs 20 --idle-secs 120 --switches 40`, **PR 2** (`exec`, `push` and
`button`, binary sha256 `0fdf25a6...`) against **this PR** (`5aec9c2d...`), four
runs of each in alternating order (2 and 2 with the order swapped), so the
rows are the cost of this entry alone. **Dev VM numbers, not the ratchet's
machine; the run that counts is the maintainer's and is still to do.**

| Row | PR 2 (4 runs) | this PR (4 runs) | `compare` (per run) |
| --- | --- | --- | --- |
| Size, stripped binary + non-glibc closure | 1,579,720 | 1,645,256 (+4.1%) | same |
| Bare executable (not gated) | 1,446,624 | 1,512,160 (+4.5%) | |
| `.text` / `.rodata` / `.eh_frame` | 1,076,328 / 104,191 / 101,252 | 1,110,728 / 106,527 / 103,484 | +3.2% of `.text` |
| Idle RSS, PSS | 3.5 / 1.9 MiB in every run | 3.5 / 1.9 in every run | same x4 |
| Idle wakeups per minute | 2 x4 | 2 x4 | same x4 |
| Idle CPU, 120 s window | 0.8, 0.8, 1.1, 1.2 ms | 1.1, 1.1, 1.2, 0.8 ms | regressed, regressed, regressed, better |
| CPU while switching workspaces | 4.1, 3.5, 4.7, 4.1 ms | 4.1, 4.2, 4.6, 4.2 ms | same, regressed, same, same |
| Startup to first frame | 33.9, 40.6, 38.7, 30.5 ms | 32.0, 37.7, 32.2, 35.2 ms | same x4 |

- **The CPU rows are noise, not a cost.** They are millisecond counts in a
  120-second window (about 2 wakeups), and this PR's idle loop adds two
  branches a turn (`pump_events`, `events_timeout`); the same PR 2 binary
  measured 0.8, 0.8, 1.1 and 1.2 ms in four runs (and 1.1 and 1.0 in the
  entry before this one), so a 0.3 ms difference moves both ways across runs.
  `compare` flagged four of the sixteen row-comparisons regressed (idle CPU
  in three runs, switching CPU in one) and one better, in different directions
  on different runs. **This is not waived**: the maintainer's run on the
  ratchet machine decides it, and `tests/agent.rs`
  (`an_idle_bar_with_subscribers_and_queries_wakes_for_nothing`) pins the
  property that matters, **zero wakeups over three idle seconds with two
  subscribers attached**, and zero again after one change is told.
- **The binary grew 65 KB** (`.text` 34 KB): the three new requests, the
  event pump, the layout geometry and `serde_json` writers for the reply
  types. No new dependency (`Cargo.toml` is unchanged).
- Not measured: a subscribed bar under a module changing at the frame rate
  (the integration test drives twenty `set`s in one burst and asserts at most
  twenty events and the last text; its allocation-free property is by
  construction, buffers reused, and not counted by a test).

## Evidence

On the dev VM (aarch64, 6 vCPUs), the tree in its own directory with its own
`CARGO_TARGET_DIR` and `CARGO_INCREMENTAL=0`, shipped by `tar` over `ssh`. The
integration tests ran against the VM's `scoot` (`/var/cargo-target/debug/scoot`,
built beside, ipc protocol 4) and its `sway`. The code is at `4b864337e` (the commit after
it is documentation and the backlog only). Raw results:

```text
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar --all-targets [FLAGS] -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | --no-default-features --features clock
  | ... --features workspaces | ... --features icon-image | ... --features button
  | ... --features push | ... --features exec | --all-features
cargo nextest run -p scootbar                  Summary 665 tests run: 665 passed, 0 skipped
cargo nextest run -p scootbar  (the target dir's scoot moved away)    665 passed (the integration tests skip)
SCOOTBAR_REQUIRE_SCOOT=1 cargo nextest run -p scootbar --test agent  (scoot moved away)   6/9 tests run: 0 passed, 6 failed (a skip is a failure when required; 3 tests need no scoot)
SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    576 + 10 + 3 + 11 + 8 + 11 + 8 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed, 0 failed
cargo nextest run -p scootbar --bin scootbar --all-features                595 passed
  ... --no-default-features 379 | clock 452 | workspaces 439 | icon-image 397
  | button 401 | push 404 | exec 421
```

`cargo nextest run --workspace` was not run: this change touches no other
crate (the diff is `crates/scootbar` and `docs/`), and `scoot-ipc` and `scoot` are
unchanged.

**The fidelity test checks what it claims**: with `layout`'s end-of-span
rounding changed from rounding up to rounding down, `layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales`
failed (the mutation was reverted; the diff against the commit is empty). It
asserts the two outputs are at scales 1 and 1.5.

The fidelity guarantee is pinned two ways: that test clicks, through scoot's own
input injection, the first and last logical pixel of every module's rectangle
(the binding runs, and it is the right module's) and one pixel outside either
side (it does not), on two outputs; and a pure test checks that for every
device pixel of a span at eight scales, its logical extent is inside the
rectangle (`daemon/agent/tests.rs`). `query`'s text is checked against the
screen in `tests/exec.rs` (the same state).

Not verified: real hardware (no `--tty` run: nothing here touches the DRM or
the VT path), the ratchet on the maintainer's machine, `subscribe` against a
compositor other than scoot headless and sway (the `output` events are checked
on sway, `module` events on scoot), and a subscriber under a sustained
frame-rate stream.
