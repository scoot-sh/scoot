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
to do.** *Update 2026-10-01: measured on the Asahi M2 since: see
[the M4 section](../../README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2)
and #368. The stack against M3 post-fix, `compare` exit 1 at every layer: size
+9.5% (#364), +14.2% (#366), +19.0% (#367), idle RSS/PSS/peak +0.14 to +0.20
MiB; startup, wakeups, idle CPU (pooled) and the switching CPU unchanged;
nothing waived.*

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
  layout the last committed frame used, in device pixels) converted at the
  output's current scale (agent.rs `write_layout` reads `output.scale()`; it is
  not recorded with the frame), rounded outward (`Scale::logical_floor` / `logical_ceil`), so a
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
`button`, binary sha256 `0fdf25a6...`) against **this PR**, so the rows are the
cost of this entry alone. Runs 1 to 4 are of the first build of this PR
(`5aec9c2d...`, code `2a138f0e`), in two orders; runs 5 and 6 are of the final
build (`d925d11b...`, code `883a9039`, which fixes a subscriber bug and streams
`layout`; the idle path is the same two calls a turn), pr3 first then pr2.
**Dev VM numbers, not the ratchet's machine; the run that counts is the
maintainer's and is still to do** (done since: see the update at the top).

| Row | PR 2 (6 runs) | this PR (6 runs) | `compare` (per run) |
| --- | --- | --- | --- |
| Size, stripped binary + non-glibc closure | 1,579,720 | 1,645,256 (+4.1%), both builds | same x6 |
| Bare executable (not gated) | 1,446,624 | 1,512,160 (+4.5%), both builds | |
| `.text` / `.rodata` / `.eh_frame` (final) | 1,076,328 / 104,191 / 101,252 | 1,101,512 / 106,527 / 103,300 | +2.3% of `.text` |
| Idle RSS, PSS | 3.5 / 1.9 MiB in every run | 3.5 (3.4 once) / 1.9 | same x6 |
| Idle wakeups per minute | 2 x6 | 2 x6 | same x6 |
| Idle CPU, 120 s window | 0.8, 0.8, 1.1, 1.2, 0.7, 1.1 ms | 1.1, 1.1, 1.2, 0.8, 1.0, 1.1 ms | regressed x4 of 6 runs (1, 2, 3, 5), better (4), same (6) |
| CPU while switching workspaces | 4.1, 3.5, 4.7, 4.1, 4.4, 4.3 ms | 4.1, 4.2, 4.6, 4.2, 3.9, 3.7 ms | same x4, regressed (2), better x2 (5, 6) |
| Startup to first frame | 33.9, 40.6, 38.7, 30.5, 39.3, 40.6 ms | 32.0, 37.7, 32.2, 35.2, 27.7, 31.0 ms | same x6 |

- **The CPU rows are noise, not a cost.** They are millisecond counts in a
  120-second window (about 2 wakeups), and this PR's idle loop adds two calls a
  turn (`pump_events`, which returns after one comparison with no subscriber,
  and `events_timeout`). The same PR 2 binary measured 0.7 to 1.2 ms in six runs
  (and 1.1 and 1.0 in the entry before this one); `compare` flagged idle CPU
  regressed in four of six runs, better in one, and the switching CPU regressed
  once and better twice. **This is not waived**: it is a finding for the
  maintainer's run on the ratchet machine, where the margin has a quieter floor,
  and `tests/agent.rs` (`an_idle_bar_with_subscribers_and_queries_wakes_for_nothing`)
  pins the property that matters, **zero wakeups over three idle seconds with two
  subscribers attached**, and zero again after one change is told. The
  difference is under the 1 ms the VM's `/proc` accounting can separate.
- **The binary grew 65 KB** (`.text` 25 KB), which the 64 KiB segment
  alignment turns into one step (both builds, whose `.text` differs by 9 KB, have
  the identical file size): the three new requests, the event pump, the layout
  geometry and `serde_json` writers for the reply types. No new dependency
  (`Cargo.toml` is unchanged).
- Not measured: a subscribed bar under a module changing at the frame rate
  (`a_stream_of_changes_is_told_at_the_frame_rate` drives a change a millisecond
  for 400 ms and bounds the events by frames; the event path's reuse of its
  buffers is by construction, not counted by an allocation test).

## Evidence

On the dev VM (aarch64, 6 vCPUs), the tree in its own directory with its own
`CARGO_TARGET_DIR` and `CARGO_INCREMENTAL=0`, shipped by `tar` over `ssh`. The
integration tests ran against the in-tree `scoot` (built beside, ipc protocol 4)
and the VM's `sway`. The code is at `883a9039` (the commit after it is
documentation and the backlog only: `git diff 883a9039 HEAD -- crates` is empty).
Raw results:

```text
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar --all-targets [FLAGS] -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | --no-default-features --features clock
  | ... --features workspaces | ... --features icon-image | ... --features button
  | ... --features push | ... --features exec | --all-features
SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1 cargo nextest run -p scootbar
                                               Summary 671 tests run: 671 passed, 0 skipped
cargo nextest run -p scootbar  (the target dir's scoot moved away)    671 passed (the integration tests skip)
SCOOTBAR_REQUIRE_SCOOT=1 cargo nextest run -p scootbar --test agent  (scoot moved away)   6/10 tests run: 0 passed, 6 failed (a skip is a failure when required; 4 tests need no scoot)
SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1 cargo test -p scootbar
                                               582 + 10 + 3 + 11 + 8 + 11 + 8 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed, 0 failed
SCOOTBAR_REQUIRE_SCOOT=1 cargo nextest run --workspace --no-fail-fast    3397 tests run: 3397 passed (1 slow), 29 skipped
cargo nextest run -p scootbar --bin scootbar --all-features                601 passed
  ... --no-default-features 385 | clock 458 | workspaces 445 | icon-image 403
  | button 407 | push 410 | exec 427
```

`scripts/backlog check` reports three problems, all under `docs/backlog/` and
all present on the exec branch too (`protocol-gaps-general`, `protocol-gaps-niche`
and `multi-output-foundation-done`); none is a scootbar entry.

**Each guard was checked against a mutation**, reverted afterwards (the diff
against the commit is empty):

| Mutation | Test that failed |
| --- | --- |
| `layout`'s end-of-span rounding changed from up to down | `layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales` |
| every module rectangle shifted 3 pixels right | the same test, by its screenshot comparison (24 s: it looks until its deadline) |
| the 16 ms gate in `pump_events` removed | `a_stream_of_changes_is_told_at_the_frame_rate` |
| a joining subscriber re-arms (the bug the first build had) | `a_subscriber_joining_does_not_clear_what_is_owed_to_one_already_there`, `the_first_subscriber_arms_with_what_is_shown_then_as_the_baseline` |

The fidelity rule is pinned three ways: the click test drives scoot's own input
injection at the first and last logical pixel of every module's rectangle (the
binding that runs is the right module's) and one pixel outside either side (it
does not), on two outputs at scales 1 and 1.5 (asserted); the screenshot
comparison reads each output's own pixels and requires ink in every rectangle
and none in a bar column outside them; and a pure test checks that for every
device pixel of a span at eight scales its logical extent is inside the
rectangle. `query`'s text is checked against the screen in `tests/exec.rs` (the
same state).

Not verified: real hardware (no `--tty` run: nothing here touches the DRM or
the VT path), the ratchet on the maintainer's machine (done since on the
Asahi M2, headless: see the update at the top), `subscribe` against a
compositor other than scoot headless and sway (the `output` events are checked
on sway, `module` events on scoot), and a subscriber under a sustained
frame-rate stream for long. A `SCOOTBAR_REQUIRE_SWAY=1` run of `tests/hotplug.rs`
(8 passed, 0 skipped, `a_subscriber_is_told_an_output_came_and_went` among them)
was recorded on the commit before the last one; `cargo test` above includes it
at the final one.

One thing in the first build that review of it found and this one fixes: a
`subscribe` re-armed the event state, so a change held back by the frame gate
was forgotten by every subscriber already attached. It is now decided by
`Events::begin` and `pending`, with unit tests.

### After review (2026-10-01)

The review's four findings, and one more found re-running the tests, are
fixed in `3f01c5ef7` and `384e78bdd` (this entry is the commit after them and
changes no code). The checks above ran at an earlier tree, so they are stale
for what these touched and were re-run at `384e78bdd`. The stack was rebased
onto `main` at `7a1f9030a` and the two PRs under this one (their own fixes).

- **`subscribe` printed a cut line as if whole and exited 0.** The daemon
  drops a subscriber whose batch the socket cannot take whole, which can leave
  the client a prefix of a line. `control::client::stream_to` now gives `each`
  whole lines only: an unterminated last line is discarded, the command says
  so on stderr and **exits 1** (a stream that closes between two lines is still
  0, which the second review below shows is not "the daemon went away"), and a line past the 16 MiB bound is refused the same way. Four tests
  (`control/tests.rs`, against a scratch socket); with the old behavior put
  back, the three that assert the new outcome fail (`a_line_cut_by_the_connection_ending_is_discarded_and_an_error`,
  `a_first_line_cut_is_discarded_too`, `a_line_with_no_end_past_the_bound_is_refused_not_passed_on`)
  and the clean-close one passes.
- **Docs**: aiming at a layout rectangle is `scoot msg pointer click X Y`
  (checked against `crates/scoot/src/cli.rs`'s own help and `docs/ipc.md`;
  there is no `scoot msg click`); `layout` converts the last committed
  frame's device-pixel spans with the output's **current** scale
  (`write_layout` reads `output.scale()`; nothing records the scale a frame was
  drawn at), which this entry and `cli.md` had overstated; `subscribe` has no
  snapshot, so "subscribe first, then query".
- **A hang in a test, found by `cargo test` (nextest never reached it).**
  `a_stream_of_changes_is_told_at_the_frame_rate` wrote about 300 `set`s in
  400 ms and never read an answer. A connection nobody reads the answers of
  stalls itself once the socket's buffer of replies is full, about 270 tiny
  replies (the kernel counts each write's overhead, not its 14 bytes): the
  daemon stops reading requests (`ss -xp` on a hung run showed 1,064 unread
  bytes of requests in the daemon's receive queue and the bar showing the 289th
  value, not the last), the last `set` is never applied and the test waits for
  the last value for ever. `cargo test -p scootbar --test agent` hung in 2 of 8
  runs, and the full `cargo test` run of the review round hung on it for 8
  minutes. A thread now drains the replies: 0 hangs and 0 failures in 55 runs
  after. The daemon's behavior is unchanged, and the code says it is deliberate:
  `control/conn.rs` stops handling a connection's requests once
  `OUT_SOFT_LIMIT` (4,096 bytes) of replies are queued and resumes when the
  client reads, and `control/mod.rs` uses no timers: at the cap of 16 the oldest
  connection (since the second review, the oldest that is not a subscriber) is
  closed to admit a new one (`admit`), which is what bounds a client that never
  reads. [robustness-and-limits](../robustness-and-limits.md)
  still lists "a write-stall deadline that drops a peer that stopped reading" as
  a bound to build: this is the case it describes, and it is not built (the hung
  run held its connection for eight minutes). Left alone here.

```text
checks at 384e78bdd (git archive of it, dev VM, own targets in /dev/shm, scoot built from
the pointer tree: no crates/scoot change anywhere in the stack, /dev/shm/m4t/debug/scoot):
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | ... --features clock | ... --features workspaces
  | ... --features button | ... --features push | ... --features exec
  | --features icon-image | ... --no-default-features --features icon-image | --all-features
SCOOTBAR_TEST_SCOOT=/dev/shm/m4t/debug/scoot SCOOTBAR_REQUIRE_SCOOT=1 \
  cargo nextest run -p scootbar --no-fail-fast        Summary 686 tests run: 686 passed, 0 skipped
cargo nextest run -p scootbar --no-fail-fast  (target dir with no scoot)   686 passed (the integration tests skip)
SCOOTBAR_TEST_SCOOT=... SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    595 + 10 + 3 + 11 + 8 + 13 + 8 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed
SCOOTBAR_REQUIRE_SWAY=1 cargo test -p scootbar --test hotplug              8 passed, 0 skipped (sway 1.12 on the VM)
cargo nextest run -p scootbar --bin scootbar --all-features                614 passed
  ... --no-default-features 391 | clock 464 | workspaces 451 | icon-image 409
  | button 413 | push 416 | exec 440
cargo check --locked --bins in crates/scootbar/fuzz                        ok
```

### After the second review (2026-10-01)

One finding in this layer, fixed in `00131b51e`, and a hardening of two test
helpers in `44935cf4d` (the code; this entry is the commit after them and
changes none). The two layers under it changed too, so this one was rebased
onto them and everything below was captured at `44935cf4d`. **The SHAs in the
earlier entries are those their checks were captured at, before this rebase**:
`3f01c5ef7` is now `52001d4b1`, `384e78bdd` is `84b00ff79` and `b6dee9710` is
`56249798a`.

- **`msg subscribe` exit 0 does not mean "the daemon went away"** (the first
  review's fix, `3f01c5ef7`, claimed it did: in `--help`, `cli.md`, the
  `Error::Cut` text and its commit). Measured by the second review: a
  subscriber the daemon drops by closing it on a full socket (`send_event`
  failing on a write that took nothing, `WouldBlock` with 0 bytes), or by the
  cap's `conns.remove(0)` (no exemption for subscribers), ends on a clean
  newline with exit 0 and nothing on stderr; only a drop mid-write gave 1.
  So a script could not tell the two apart. Fixed in what can be, documented
  in what cannot:
  - **Eviction keeps subscribers.** Closing a client to admit another (the
    cap, or freeing a descriptor on `EMFILE`) now takes the oldest
    *non-subscriber*; with `MAX_SUBSCRIBERS` (4) under `MAX_CONNECTIONS` (16)
    there always is one at the cap (a `const` assertion says so), so a flood of
    connects no longer ages out an agent's subscription. Only with nothing but
    subscribers to close (the `EMFILE` path) does one go.
  - **`{"type":"dropped"}`.** A subscriber that is closed that way gets that
    line as its last, by one nonblocking write that is never queued;
    `msg subscribe` prints it and **exits 1**, saying so on stderr. It cannot be
    sent for the common drop: a subscriber that stopped reading has a full
    socket (a 19-byte write fails exactly where the batch did), and the daemon
    never blocks or buffers for it. That drop still ends with no line, and
    **exit 0 can still mean it**, as can a partial write that happened to stop
    on a newline. Said so in `--help`, `cli.md` (a new "how a subscription
    ends" list and an exit-status row) and the code comments; after any end,
    exit 0 included, subscribe again and `query`.
  - *For the coordinating session:* the robust form of "the daemon went away"
    is a positive end-of-stream line sent on clean shutdown (`close_all`
    already makes one non-blocking attempt per client), with its absence
    meaning "resubscribe". Not done here: it is a protocol addition, not a
    fix of this finding.
  - Tests: `control/subscribe_tests.rs` (a flood closes idle clients and never a
    subscriber; the oldest *idle* goes first; with only subscribers left the
    oldest is told `dropped` whole as its last line and the rest are untouched;
    an idle client is told nothing; a full socket is not waited on),
    `control/tests.rs` (a `dropped` line is passed on, then an error),
    `tests/agent.rs` (a subscription on a live bar outlives 48 idle connections
    and still hears the next change) and the new `tests/subscribe_status.rs`
    (`msg subscribe` against a socket that writes what it is told: `dropped`
    printed and exit 1, a cut line exit 1, a plain end exit 0 with nothing said).
    With the exemption removed, three of them fail (the two unit tests and the
    live-bar one); with the notice removed, the one for it fails; checked and
    reverted.
  - The `broadcast` and `send_event` paths gain no work on the success path, and
    only the failure path changed in `admit`/`free_an_fd`, so no benchmark was
    rerun: the loop with no subscriber is still one branch, and an event is still
    one write per subscriber.
- **A test helper that could hang instead of fail** (`44935cf4d`): the fake
  daemon of `control::tests::streamed` and of `tests/subscribe_status.rs` blocks
  in `accept` inside a `thread::scope` that waits for it, so a client that never
  connected (a path error) would hang the test. One connection to the still-open
  listener after the client is done lets the thread end.
- **Other tests that assume a clean launcher: audited** (the grep and its
  result are in the pointer layer's entry, whose layer this layer sits on): the
  new `tests/subscribe_status.rs` needs no compositor and reads no environment
  but the two variables it sets, and the whole suite passes from `/` under
  `TZ=Asia/Kathmandu`, `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`/`WAYLAND_DEBUG` set,
  no `HOME` or `XDG_RUNTIME_DIR`, the open descriptors and the ignored `SIGHUP`.

```text
checks captured at 44935cf4d (git archive, fresh extract), dev VM, scoot built from the stack's
tree (/dev/shm/scoot-bin, built at 47b413a99: nothing in the stack changes crates/scoot*),
own targets in /dev/shm; log: verify-agent-44935cf4d.log
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | ... --features clock | ... --features workspaces
  | ... --features icon-image | ... --features button | ... --features push | ... --features exec
  | --all-features
SCOOTBAR_TEST_SCOOT=/dev/shm/scoot-bin SCOOTBAR_REQUIRE_SCOOT=1 cargo nextest run -p scootbar
  plain                                                  Summary 709 tests run: 709 passed, 0 skipped
  hostile (fds 4 5 142 145 open, SIGHUP ignored)         709 passed
  under nohup                                            709 passed
cargo test -p scootbar, plain and hostile                all "test result: ok" (611 in the bin; 5 more hostile runs of the bin: 611 each)
from /, TZ=Asia/Kathmandu, WAYLAND_* set, no HOME, hostile   709 passed (nextest), all ok (cargo test)
nextest, target dir with no scoot, hostile               709 passed (the integration tests skip)
hostile nextest --bin scootbar: --all-features 630 | --no-default-features 403
  | clock 476 | workspaces 463 | icon-image 421 | button 425 | push 428 | exec 456
SCOOTBAR_REQUIRE_SWAY=1 cargo test -p scootbar --test hotplug          8 passed (plain and hostile; sway on the VM)
bash -c 'exec 4</dev/null 5</dev/null 142</dev/null 145</dev/null; exec <bin> <test> --exact'
  for spawn::tests::a_child_inherits_none_of_the_callers_descriptors and
  modules::exec::tests::a_child_holds_none_of_the_callers_descriptors   ok, ok (the lead's literal form, as written)
cargo check --locked --bins in crates/scootbar/fuzz                    ok
(captured earlier, at the pre-rebase ff9032ba0, which differs from 44935cf4d in the two layers under it and the
 fake-daemon helper change: the mutation checks above, and verify-agent-ff9032ba0.log)
```

Not verified: a daemon out of file descriptors with only subscribers open (the
`EMFILE` path that sends `dropped`) on a live bar: it is tested on the server with
socket pairs (`with_only_subscribers_left_to_close_the_oldest_is_told_it_was_dropped`),
not by exhausting a real process's limit; `nix build` (no `nix/` file or
dependency changed); real hardware.
