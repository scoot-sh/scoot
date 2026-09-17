---
title: "Screenshot capture and PNG encode run on the sole event-loop thread (MEDIUM) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Screenshot capture and PNG encode run on the sole event-loop thread (MEDIUM) — RESOLVED

Split out of
[the connection-cap entry](../resolved/ipc-connection-cap-resolved.md) when
its other two concerns were closed (2026-09-16); this is the one that was
always going to be its own piece of work, and it is unaffected by both fixes
that landed there.

## What it said

A `Request::Screenshot` costs a full render, a framebuffer read-back and a
PNG encode, all on the compositor's only thread — the one that also runs
wayland dispatch, input and every other IPC connection. Each capture stalls
all of that for its duration: **~12ms at 1600x1000 in a release build**,
measured in item 9.

What was already done and did *not* close it:

- **The per-connection rate limit** (item 9): one capture per connection per
  16ms frame, refused rather than delayed. It bounds how often the stall can
  happen, not how long one lasts.
- **The connection cap** (`ipc/slots.rs`): 64 connections, so the rate limit
  can no longer be multiplied by reconnecting. Same thing — a bound on
  frequency, not on the stall itself. Worth noting for whoever picks this up:
  64 connections each allowed a capture per frame is still, in the limit, far
  more capture work than one thread can do in a frame, so the cap makes the
  worst case finite rather than acceptable.

## Resolution (2026-09-17)

**The decision: a single dedicated worker thread, not a pool and not async.**
Render and read-back stay on the event-loop thread — they touch the renderer
and its framebuffer, which are not `Send`, and moving them would mean
restructuring who owns the backend. The BGRA-to-RGBA swizzle, the PNG encode
and the reply's JSON/base64 framing move to the worker, which are pure CPU
over an owned `Vec<u8>`. A single FIFO was chosen over a pool for one
reason: one worker preserves reply order with no sequencing machinery, and
per-connection order is the guarantee the protocol keeps (see below).

**Reply delivery** reuses the mechanism this socket already has for an answer
that outlives `serve`: the `PendingIdle` hand-off. Dispatch clones the
connection's socket (non-blocking, like the original) and parks the
reply-to-be in `State::pending_shots`; the worker's framed line comes back
over a calloop channel whose callback runs on the event-loop thread and
writes it through that clone with the same `Outbound` queue a connection
uses. No second reply channel. A reply the socket will not take in one go
drains from the frame tick, with the same progress-not-total-time give-up
the other two write paths have (10s, mirroring `WRITE_STALL_TIMEOUT`).

**Ordering: strict per-connection.** A connection with a capture in flight
answers nothing else until the capture's reply has gone out -- any other
request meanwhile is refused with a retry, the "refused rather than delayed"
shape the rate limit already has. That includes `wait-idle` (parking behind
a capture would leave the capture's answer arriving on a handed-over
connection) and a second screenshot past the rate-limit window. A capture is
also refused while its connection's own earlier replies are still queued:
the completion writes through a socket clone and would otherwise land
mid-queue. One visible consequence, documented in `README.md`: the refusal
to a throttled second screenshot is answered immediately, so it arrives
*before* the first capture's reply -- pipelining clients match those two by
content, not position. `flexwm msg` sends one request per connection, so it
never meets the ordering gate -- but concurrent `msg screenshot` processes
can meet the 4-global busy refusal below, which surfaces as an ordinary
error (non-zero exit, no retry inside `msg`), consistent with the
rate-limit refusal.

**Disconnect mid-encode** holds nothing borrowed and no slot: the completion
write fails against the dead peer and the entry is dropped. Pinned by a
harness test and hammered end to end (20 clients killed mid-screenshot in
`scripts/smoke-test.sh`).

**`wait-idle` semantics: unchanged.** The capture's render touches only
`needs_render`, never `last_commit` — the clock `wait-idle` watches — and
the encode in flight parks no waiter. A capture mid-wait neither extends nor
shortens the quiet window, pinned by asserting `last_commit` is unmoved
across a capture plus an end-to-end idle answer beside one.

**Bounded memory: four captures in flight, across every client.** Each holds
a full frame of raw pixels (~6.4 MiB at 1600x1000); past four the next
capture is refused with a retry rather than queued without bound.

**Panic isolation is structural, not `catch_unwind`.** Release builds with
`panic = "abort"`, under which nothing can be caught — so `encode_png` is
written with no panic path (checked arithmetic, no indexing past
`chunks_exact`, every fallible call mapped to an error string). The
`catch_unwind` around the job still stands for debug builds. If the worker
ever goes away entirely, `try_send` fails disconnected, which drops the
encoder so the next request spawns a fresh one; the current request is
refused with a retry, never hung. The worker is never joined: it exits on
its own when `State` (and its job channel) goes away, so shutdown never
waits on an encode.

**Benchmarked** on the dev VM (release, `--headless` 1600x1000, per-op wall
clock over `flexwm msg`, so every figure below includes ~5ms of spawn and
connect on both sides — the deltas are the signal):

| metric (ms, median with spread) | before | after |
| --- | --- | --- |
| idle `version` | 6 (4–8) | 6 (4–6) |
| `screenshot` round trip | ~18 (13–21) | ~17 (15–20) |
| `version` during a 1-connection capture flood | ~11 (8–13) | ~8 (6–13) |
| `version` during an 8-connection capture burst | not measured | ~19 (11–29) |
| screenshot PNG bytes | 33,371 | byte-identical (`cmp` clean) |

Read it as: the per-capture event-loop stall is the render and read-back
that stayed (~2ms: version-under-load minus idle, 5ms → 2ms), and the
encode's ~10ms no longer blocks anyone — total screenshot latency is
unchanged, as it should be, since the work still happens. Under an 8-way
burst the bound is one round of renders rather than eight encodes stacked
end to end; the 8-way before-side was not re-measured (the baseline binary
was overwritten by the branch build before the burst probe existed — the
1-way A/B pair plus the serialization mechanism is the evidence there).
The burst-after figure above was taken against the pre-framing binary; moving
the framing to the worker afterwards only removes sub-millisecond loop work
(the 33 KB PNG's base64), so it stands.

**Bug-bash findings from the diff's own review**, both fixed before merge:
a capture dispatched over a connection's still-queued replies would have its
completion land mid-queue (now refused with a retry, pinned by
`a_screenshot_waits_for_replies_still_going_out`), and the reply's
JSON/base64 framing was first encoded on the event-loop thread (now framed
on the worker; the loop never touches PNG bytes). The smoke script's first
version of the kill-mid-screenshot section hung on a bare `wait` that also
waited for the compositor child — now waits only on the killed clients'
pids, with a comment saying why.

## Adjacent, named rather than fixed here

The 4-global cap's starvation shape belongs with the entry the cap work
already filed: four slow or non-reading consumers can deny screenshots to
everyone else -- each drain cycle giving up only after its own 10s
no-progress window -- which is the same shared-table class as [what a
shared connection table costs an innocent
client](../ipc/connection-cap-denies-the-same-user.md). Cited there rather
than fixed here: the bound is doing its job (finite memory, refused not
queued), and the unfairness is the documented price of sharing it.
[The accept loop and `EMFILE`](../ipc/accept-loop-swallows-emfile.md) from
the same review stays open too.
