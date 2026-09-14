---
item: "11"
title: "Flush a real keystroke to the client without waiting"
status: "done"
area: "input"
pr: 17
commit: null
---

# Flush a real keystroke to the client without waiting

**The fix is one line in `compositor/mod.rs`**: `event_loop.run`'s
post-dispatch callback was `|_| {}` and is now `post_dispatch`, which calls
`display_handle.flush_clients()`. That makes "anything queued during a
wakeup goes out at the end of that wakeup" structural rather than a
discipline to remember at each call site -- which is what had been missed:
`tty/mod.rs`'s libinput callback and `nested_dispatch.rs`'s host-forwarded
equivalent both hand a real key to `input::key`, which queues the client's
`wl_keyboard.key` and nothing else. A keystroke does not mark the screen
dirty, so `render()` early-returns on `!needs_render` before its own flush,
and the frame timer has already dropped itself when idle.

Three claims behind the suggested fix were checked rather than taken as
given, and one needs restating:

- **calloop 0.14.4's `EventLoop::run`** is literally `while !stop {
  dispatch(timeout)?; cb(data); }` (`loop_logic.rs:657`), `cb: FnMut(&mut
  Data)`. So the callback runs once per wakeup -- and with flexwm's
  `timeout` of `None`, `dispatch` blocks until a source is ready, so an idle
  compositor gets no wakeups and therefore no flushes. That is the whole
  idle-CPU argument.
- **"Matching anvil's pattern" is true in substance, not in form.** Anvil
  (`anvil/src/udev.rs:520-545`, `winit.rs`, `x11.rs`) does not use `run` at
  all: it hand-rolls `while running { dispatch(Some(1ms|16ms)); space
  .refresh(); popups.cleanup(); flush_clients().unwrap() }`. Same
  flush-after-every-dispatch-cycle shape at the hook `run` provides
  instead; flexwm's `space.refresh()`/`popups.cleanup()` stay in `render()`,
  and unlike anvil's timeout-driven loop, `run(None, ..)` does not poll at
  idle.
- **The cost of a flush with nothing queued**, re-derived from
  wayland-backend 0.3.17 rather than from the review's characterization of
  it: `flush_clients` → handle `flush(None)` → `for client in clients_mut()
  { let _ = client.flush() }` (`rs/server_impl/handle.rs:63-74`) →
  `BufferedSocket::flush`, whose write loop is `while written_bytes <
  bytes.len()` (`rs/socket.rs:155`) -- zero syscalls on an empty buffer,
  then `offset(0)`/`move_to_front()`/`drain(..0)`. So it is one mutex plus
  O(clients) of bookkeeping per wakeup.

**`ipc/connection.rs` is deliberately untouched**, including its
hand-maintained "every exit reaches the one flush" invariant, which this
change does make redundant for user-visible staleness. It is well-tested,
cost three review rounds, and each of the older flush sites still flushes
*earlier within its own wakeup* than this one does (the display source
before the loop moves to another source, `render()` after a frame's frame
callbacks, `step()` before a reply's own round trip can race it) -- so the
overlap buys lower latency for free. What is now stale is only the
*justification* written at two of them ("nothing else flushes until ..."),
recorded here rather than edited in: something else does now, just later.
`post_dispatch`'s own doc comment names all three and says why each stays.

**Tests: 2 new (177 total, against 175 on the merge base)**, in a new
`compositor/tests.rs`. Both drive a real `State` with a real wayland client
through a real `EventLoop::run` -- the only thing that can observe *when* a
queued message leaves, and the reason `run` is called with the same
`post_dispatch` function production uses rather than a copy of it. The
client is a raw socket handed to `insert_client` with one hand-written
`get_registry`, and the queued message is a `wl_registry.global` from a
newly created output: item 10's `connection/tests.rs` established that
fixture for the same reason (a keystroke needs keyboard focus, a mapped
toplevel and a real toolkit; what is under test is the flush, not what
filled the buffer). `Timer::immediate` + `loop_signal.stop()` is what lets a
`run(None, ..)` return after exactly one cycle -- the stop flag is only
checked *after* the callback, so the flush under test does happen. The
second test asserts a dispatch cycle *without* the callback leaves the
message queued, which is what keeps the first non-vacuous, and the
**negative control** confirms it directly: passing `|_| {}` (the production
code as it shipped) fails the first test with "the message queued before the
dispatch cycle never reached the client" and leaves the second passing.

**Hardware-verified on the dev VM's real `virtio-gpu` KMS device at
1600x1000**, release builds of `main` at `9cf8b9e` and this branch at
`0de11d2` built from `git archive` into separate trees and separate
`CARGO_TARGET_DIR`s (the staleness hazard in `HANDOFF.md`), with a real
`/dev/uinput` keyboard injecting one `KEY_X` and holding the virtual device
alive afterwards so the key is the *only* libinput event in the measured
window. Quiet-screen control first: two captures 1.5s apart with no input
at all, `magick compare -metric AE` = 0. Then, after the key:

| | capture at ~40ms | capture at ~1.5s |
|---|---|---|
| `--tty` before | AE 0 (not there yet) | AE 217.238 |
| `--tty` after | **AE 217.238** | AE 217.238 (nothing changed since) |
| `--nested` before | AE 0 | AE 217.238 |
| `--nested` after | **AE 217.238** | AE 217.238 |

The same 217 pixels either way -- the one `x` foot drew -- so the keystroke
was never lost, only late, and "late" was until the *next* screenshot's own
end-of-wakeup flush delivered it. `--nested` ran under `cage` on the same
real DRM device, with the host forwarding the injected key, which is the
other input path the Backlog entry names; it is the same event loop
(`nested.rs` inserts `WaylandSource` into the same `loop_handle`, and
`mod.rs`'s `run` is the compositor's only *production* `run`/`dispatch`
call site -- the test modules have their own, deliberately), so no separate
render loop can bypass this.

**Benchmarked**, because this runs on every wakeup. Raw numbers, all
interleaved with the run order balanced (item 10's correction):

- **Idle CPU, the concern that matters most: 0 jiffies over 60s, both
  binaries, twice each**, with a mapped `foot` and a settled screen. `run`
  blocks in `dispatch(None)` at idle, so there is nothing to flush and
  nothing to wake for.
- **A real 1000 Hz-class pointer-motion flood — corrected by
  `flexwm-reviewer` after the original benchmark below turned out to
  measure nothing.** The output is 1600x1000 and the original run parked
  the pointer at the *output* centre, `800,500` -- 6px outside the
  single test window's actual bounds (`x ∈ [12,794]`). `strace -c -f` on
  the client during a 3000-event flood at that position recorded **zero
  syscalls in 12s**: the compositor queued it nothing, so `post_dispatch`'s
  flush was an empty-client-list walk in *both* builds, and the "two
  independent sets disagree in sign" result below was an artifact of
  whichever binary happened to run first in a group being slower --
  not a real before/after difference. Re-run with the pointer verifiably
  inside the window (`400,500`), untraced, balanced run order, 10,000
  events, measuring on-CPU time directly (`/proc/<pid>/task/*/schedstat`,
  nanosecond resolution, not 10ms jiffies): compositor on-CPU **before
  mean 884.0ms (sd 59.6, n=7), after mean 1023.3ms (sd 63.9, n=7) -- a
  +139.3ms/10k-events (+13.9us/event, +15.8%) increase, Welch t=4.22,
  p≈0.001**. Client on-CPU rose ~21.7% (p≈0.02) over the same runs.
  **This is real, not noise, and it's real for exactly the reason the
  original bullet named**: motion used to batch into the frame tick's
  flush; now every wakeup's events go out at the end of it. Client
  wakeups confirm the mechanism directly (`strace -c` on `foot`, 3000
  events): `epoll_pwait` 465 → 2696 (5.8x), `recvmsg` 930 → 5424.
  **In absolute terms this is small**: at the ~435 events/s this flood
  actually delivered, that's +6.0ms compositor CPU per second of
  sustained flooding, ≈0.6% of one core -- roughly +1.4% extrapolated to
  a real 1000Hz mouse. Verdict: an accepted, real cost in exchange for
  lower per-event latency, matching Smithay's own `anvil` reference
  compositor's pattern (verified directly against the pinned checkout:
  `anvil/src/udev.rs:537-546` and `winit.rs:449-456` both hand-roll
  `dispatch(...)` → `flush_clients()` once per loop iteration, same
  shape as this fix). Caveats worth keeping in mind before citing these
  numbers elsewhere: this VM's syscalls are more expensive than real
  hardware's, so 13.9us/event is likely a ceiling, not what real
  hardware would show; the 5.8x client-wakeup multiplier is
  hardware-independent and won't shrink; and "after" runs finished ~5%
  faster in wall time across the whole benchmark (an unexplained VM
  scheduling artifact) in a direction that would bias the measured
  *increase* downward, so +15.8% is if anything an underestimate, not
  an overestimate. (The `libwayland`/`wl_display_run` comparison in the
  original bullet was not independently verified -- treat it as
  unconfirmed, not as corroborating evidence.)
- **What that costs the client, since the compositor's own jiffies cannot
  show it**: see the wakeup-multiplier numbers just above (5.8x more
  `epoll_pwait`/`recvmsg` calls under flood) -- the original "0 jiffies
  over 20,000 motion events in both builds" claim here shared the same
  out-of-window fixture bug and measured the same empty-client-list
  walk, not real client-side cost.
- **The same flood at maximum rate** (50,000 events in ~300ms, ~160k/s):
  before 7/10 jiffies, after 6/11, from the same original benchmark run
  as the corrected bullet above -- likely shares its out-of-window
  fixture bug and has **not** been independently re-verified. Treat this
  specific number as unconfirmed rather than as evidence the cost
  vanishes at high rates; the corrected, verified numbers are the ones
  above.
- **IPC round-trips** (50,000 `version` requests, one wakeup each), with
  three `foot` clients connected so the flush actually walks a client list:
  before mean 126.22us/71.3 jiffies, after 125.01us/69.8 -- after slightly
  *faster*, i.e. noise. With no wayland clients at all: 125.12us/69.8
  before, 124.75us/68.8 after.

**Bug-bashed on the same hardware**, fixed binary, all six scenarios alive
with no panic and no log line beyond smithay's pre-existing
`Failed to destroy old mode property blob` WARN at modeset: a real key with
**zero windows** (no client to flush to at all; still answers IPC and
renders afterwards); the focused client **`kill -9`'d 32ms after the key**,
i.e. around the flush, after which a fresh client is served normally;
**2,000 real press+release pairs in 21ms** (3 jiffies, 20,748 pixels
changed); a key arriving while the session is **VT-paused** (no `EPERM`,
still answers IPC, and after `chvt 1` it renders again and a fresh key
changes the screen at the fast timing); **`type` + `wait-idle` pipelined in
one 100-byte write** (both replies, 504 pixels changed by the time the idle
answer came back -- item 10's invariant unregressed); and **three clients
with one `kill -9`'d** (2 windows left, a real key still reaches the focused
one).

Traced against the frame-tick machinery it sits next to, since they are
easy to confuse: `post_dispatch` only flushes. It never touches
`needs_render` or `timer_armed`, so it cannot cause a render, keep the frame
timer alive, or interact with `ensure_ticking`/`frame_tick`'s
drop-when-idle logic -- which the 0-jiffy idle measurement confirms
empirically. `flush_clients` is also the wayland-*server* side only: the
`--nested` host connection and `wait-idle`'s own cloned-fd writes are
untouched by it.

No README change: nothing user-facing moved -- no flag, config key, request
or documented behavior -- and the Status section describes capabilities, not
latency bugs it no longer has.
