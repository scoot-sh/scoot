---
item: "10"
title: "Non-blocking IPC connection loop"
status: "done"
area: "ipc"
pr: null
commit: null
---

# Non-blocking IPC connection loop

**What shipped**, by symptom:

- **(a) a half-written request line.** The accepted socket is now
  non-blocking, and `ipc/line.rs`'s reader owns its line buffer rather
  than borrowing one from the caller — because the two are coupled in a
  way that is easy to get wrong: a line split across reads must *keep*
  its prefix, so the buffer can be cleared exactly once per line, after
  the caller has consumed it, never once per read. `WouldBlock` became
  `LineRead::Incomplete`, a third outcome distinct from both an error and
  a real end of stream, so a client that goes quiet mid-line is never
  confused with one that went away mid-line (the latter still gets that
  last line answered, as before). The 1 MiB cap now applies across
  however many reads a line arrives in — the test for that is the one
  that would have caught preserving the prefix as a way around the cap.
- **(b) a pipelined second request.** `Connection::step` answers every
  line already in its read buffer before handing the thread back. That half
  is forced: lines already pulled into userspace are never reported again by
  a level-triggered source, so leaving one there is the bug itself. What
  bounds the other side -- how long one connection may hold the only thread
  the compositor has -- is a budget of *reads off the socket* per wakeup
  (`READS_PER_WAKEUP`, one), which is the only thing that can. See the
  blocking correction below for why, and for the 32ms stall two earlier
  versions of this shipped with.
- **(c) a client that never reads its replies.** `ipc/outbound.rs` queues
  whatever of a reply the socket would not take and finishes it when the
  event loop reports the socket writable. A connection with anything
  queued is registered for writability *only*, which is the flow control:
  it cannot queue more work while it is behind, and a level-triggered
  READ registration on bytes it has decided not to read would spin the
  loop at full speed.

**Design decisions worth the reviewer's attention**, since the Backlog
entry's fix direction left them open:

- **The outbound cap is a high-water mark, not a limit, and nothing is
  ever refused, truncated or closed for being slow.** The read side's
  1 MiB number is not reusable here: a request's size is client-chosen
  and nothing legitimate needs a megabyte, but a *response* is sized by
  the compositor — a `Response::Screenshot` is legitimately several MB of
  base64 PNG, and a client reading it slowly on a loaded machine is slow,
  not hostile. So `HIGH_WATER_BYTES` (1 MiB, its own constant with its own
  rationale) only bounds what a *single wakeup* may add to the queue,
  which is the one thing the write-only interest cannot bound: the
  requests that wakeup is working through have already been read. Net
  bound per connection: 1 MiB plus the one response that crossed the mark
  — and that response was already in memory to be written, so this costs
  no more than the `write_all` it replaces, which held the same bytes for
  as long as it blocked. No timeout, and no "close a slow client", for
  the same reason.
- **The consequence, stated plainly:** a client that pipelines more
  unread requests than its own `SO_SNDBUF` holds, while never reading the
  answers, stalls — itself, alone. That is the same threshold at which
  the blocking code deadlocked, except it used to take the whole
  compositor with it. Demonstrated on hardware (below) rather than
  reasoned about.
- **`Connection::closing`** (set by end-of-stream and by an over-long
  line) means "no further request will be read", *not* "close now": the
  connection stays in the loop until its queue has drained. Without that,
  the end of a client's requests would also be where its replies got
  discarded — a client that writes a request, shuts its write half down
  and waits for the answer is a perfectly ordinary client.
- **`wait-idle`'s hand-off takes the connection's outbound queue with
  it.** That request removes its own source from the event loop and
  answers later from the render loop through a cloned fd, so anything
  still queued on the connection would have nothing left to write it —
  and the idle answer, written through the same socket, would land in the
  middle of a half-written response. The `PendingIdle` carries the queue,
  and its own write is non-blocking too (`try_clone` shares file status
  flags, verified), retried per frame tick, given up on only after the
  client's own `timeout_ms` has passed with *no write progress at all*
  rather than after a fixed deadline — a client draining a multi-megabyte
  reply slowly is making progress and is never given up on. The cost of
  that retry, named because this project benchmarks idle CPU: while such a
  reply cannot drain, `frame_tick` keeps rescheduling at 16ms (`render()`
  early-returns on `!needs_render`; the retry is one `EAGAIN` write per
  tick) for up to that `timeout_ms`. Not a regression — a `wait-idle` with
  a huge timeout over a never-settling screen already held the timer
  exactly the same way — and bounded by what the client itself asked for.
- **`ConnectionSource`**, a small `EventSource` wrapping `Generic`, exists
  because `Generic`'s callback cannot reach the `Generic`'s own `interest`
  field, and switching interest from inside the connection's own callback
  is the whole point. Both halves are needed and in this order: set the
  field (what `reregister` reads) and return `PostAction::Reregister`
  (what gets `reregister` called). Checked against calloop 0.14.4's own
  source — `loop_logic.rs`'s post-callback handling, `generic.rs`'s
  `reregister`, and `token.rs` for the fact that the re-derived token is
  identical, so no event can be lost to the switch.

**One correction to the diagnosis below:** its "with a cap on that buffer,
since it is the same unbounded-growth shape item 9 just closed" reads as a
hard cap, which would have been wrong for screenshots -- see above.

**And one correction to two earlier versions of this entry, which
`flexwm-reviewer` caught as a blocking finding.** They claimed "one read
buffer is the bound on how long one connection can hold the thread", and
justified deleting an explicit per-request counter on the grounds that the
read buffer had been bounding a wakeup all along. **Both were false, and the
second is why the first went unnoticed.** `Lines::next` has its own loop
around `fill_buf`, because a line that ends mid-chunk can only be finished
by reading again -- so a serve loop that continues "while something is still
buffered" runs until a chunk boundary happens to land on a line boundary,
i.e. for `lcm(chunk, line length)` bytes, which for a line length coprime
with the chunk is the whole flood. Neither lines served, nor bytes served,
nor the buffer's size bounds that; only counting reads does, which is what
`READS_PER_WAKEUP` is.

Measured, because the first write-up reasoned where it should have
measured. One ordinary client doing strict request/response round-trips
while another floods with 160 KB batches of 101-byte requests; release
build, 6s samples:

| | p50 | p90 | p99 | max | round-trips |
|---|---|---|---|---|---|
| quiet (either way) | ~126us | ~155us | ~242us | 1.8ms | 23,608 |
| flooded, no read budget | **31,975us** | 34,768us | 35,988us | 38.6ms | 195 |
| flooded, one read per wakeup | **358us** | 390us | 553us | 5.6ms | 17,462 |

Two frames of frozen input, dispatch and rendering per wakeup, repeating
for as long as the flood lasts -- a materially weaker version of "only the
offending client is affected", which is the whole point of this item. The
bound costs the flooding client about 9% of its own throughput (423 MB vs
386 MB of requests pushed in 8s) and gives the innocent one 90x the
round-trips at 65x better p99. Reproduced on real `--tty` hardware to the
same figures (p50 353us, p99 553us), so it owes nothing to the backend.

The test that covers this had to be written at that scale *and* with that
line length: at 600 19-byte `version` requests it passes either way,
because this kernel hands out 2641-byte socket chunks and 19 divides 2641
exactly, so even the unbounded loop yields at the first boundary. That is
also why two rounds of review missed it -- every earlier test used 19-byte
requests.

A third claim was **confounded rather than false**: the "+2 jiffies per
50,000 round-trips" attributed to the read-until-`Incomplete` shape was
measured with a biased design (the same binary always first in a rep), and
a balanced one shows the run *position* is worth about that much on its own
(whichever binary runs second in a rep averages +1.15us and +1.36 jiffies).
The extra `EAGAIN` syscall per request was real and provable by inspection,
and removing it was right, but its cost was never measured apart from the
artifact.

**And one more finding, introduced by that very fix.** Hoisting
`flush_clients()` out of the per-line loop left two exits that skipped it:
`serve` returning `Step::Close` -- the `wait-idle` hand-off, or a reply that
could not be written -- and a failed read, both of which returned straight
out of `step`. `wait-idle` makes that a correctness bug rather than a
latency one, on exactly the pipeline an agent writes: `type` and `wait-idle`
in one write. The keystroke's wayland messages are queued by the time
`serve` returns, the early return skipped the flush, and nothing else
flushes them -- the frame timer's `render()` returns immediately unless
something marked the screen dirty, and injected input does not. So the
client could not have redrawn for a key it never received, and
`idle_outcome` answered `idle` over a screen that had not changed yet: the
stale-idle race its own doc comment is about. Every exit now breaks rather
than returns, `served` is set *before* `serve` runs (a lone `wait-idle`
serves one request and then closes), and the flush is the single point they
all pass through.

Shown on hardware rather than argued, against a release build of the same
tree with that one exit returning early again: spawn `foot`, let it settle,
capture, then one 106-byte `write()` of `type 'echo
pipelined-flush-probe'` + `wait_idle quiet_ms=300`, then capture again the
moment the idle answer comes back. `magick compare -metric AE` between the
two captures: **586 pixels changed with the fix, 0 without it** -- and 586
either way once the screen had settled, so the keystroke was never lost,
only late. `waited_ms` was ~305 in both runs: the clock cannot tell the two
apart, which is why this is measured on the screen.

**Deliberately out of scope, named because they are adjacent:**
`wait-idle` is still terminal for its connection, so a request pipelined
*after* one is silently never read (pre-existing, unchanged). A refused
over-long request is followed by `ECONNRESET` rather than a clean EOF,
because the compositor stops reading the rest of the line and Linux resets
a unix socket closed with unread data — the refusal itself still arrives
first (the kernel reports the error only once the receive queue is empty),
and a client mid-`write_all` past 1 MiB sees the write fail rather than
the refusal. Both pre-existing and both unchanged here. Capping concurrent
connections is still its own Backlog entry, now with a new lifecycle case
recorded next to it (found by `flexwm-reviewer` reviewing this item): a
half-closed, never-reading client pins a connection slot and two fds for
good -- strictly better than the pre-item-10 behavior, where that same
case froze the whole compositor, but still a live leak. See that Backlog
entry for the mechanism.

**Tests: 47 new (175 total, against 128 on the merge base).** Split per
module: `line/tests.rs` (partial lines, the cap across reads, the three
end-of-stream cases, the read budget), `outbound/tests.rs` (partial writes,
reply ordering, what the mark measures, a drained queue freeing its
buffer), `tests.rs` (the `wait-idle` answer's retry-and-give-up logic,
which nothing outside the compositor can observe: it is driven directly,
with a deliberately tiny `SO_SNDBUF` so the answer cannot go out), and
`connection/tests.rs`, which drives a real `State` through a real
`EventLoop` for all three symptoms plus the per-wakeup bound, the
`wait-idle` hand-off, the screenshot limiter across a pipelined write, the
graceful close, the interest registration and the per-wakeup wayland flush.
That last one needs a wayland client to observe, and uses the cheapest thing
that can be one: a raw socket handed to `insert_client`, one `get_registry`
written by hand, and then a new global created to queue a
`wl_registry.global` for it -- which nothing flushes until something
chooses to. A single `wait-idle`, with nothing pipelined ahead of it, is
then the only thing that can have flushed it. (A keystroke would be the
more literal fixture, but it needs keyboard focus, i.e. a mapped toplevel
and a real toolkit client; what is under test is the flush, not what filled
the buffer, and the hardware run above covers the literal case.) Those hand
`accept()` one end
of a socket pair rather than going through the listener -- the peer is then
this process (so the uid check passes) and, the reason that matters,
`SO_SNDBUF` can be shrunk on the compositor's end before it is handed over,
which is the only way a test makes a reply not fit in one write without
pushing megabytes through a debug build. The client and compositor share
one thread, so a test that *would* block hangs rather than fails -- which
is the correct outcome, and is what the negative controls below exercise.

**Thirteen negative controls, each a one-line mutation, run on the dev VM;
every one failed a test that passes against the real code**, which is what
makes the suite non-vacuous rather than merely green. Leaving the accepted
socket blocking hangs the partial-line test, and separately the
never-reads test, past 60s instead of failing -- exactly symptoms (a) and
(c). Clearing the line buffer on `WouldBlock` fails five tests. Serving one
line per wakeup fails the pipelining and screenshot-limiter tests with
"only 1 of 2 replies arrived" -- exactly symptom (b). Handing `wait-idle` an
empty queue instead of the connection's fails the framing test at 7 of 301
replies. Closing on end-of-stream without draining loses 305 of 400 queued
replies. Removing the read budget answers 278 of 1,622 pipelined requests
in one wakeup, against a bound of 82. Keeping a big line buffer instead of
freeing it leaves 131,072 bytes alive after the line that needed them. And
each of three independent ways of breaking the `wait-idle` write-progress
logic -- give up on the first stalled tick, never record progress, never
start the clock when the answer is queued -- fails between one and three of
the four tests written for it, including both mutations the review found
passing everything. And the two ways of losing the per-wakeup flush --
returning from the serve loop instead of breaking out of it, and marking a
request served only after `serve` returns -- each fail the flush test and,
run against the whole suite, only that test (174 passed, 1 failed, both
times), which is what makes both halves of that fix load-bearing rather than
one half plus a tidy-up. Raw output in PR #15.

One change is deliberately **not** covered by a test, and says so in its
comment: resetting `sent` alongside emptying the buffer in
`Outbound::send` guards an invariant that holds today, so nothing can
distinguish it behaviourally -- the `debug_assert` in `pending()` is what
would catch a future violation.

**Hardware-verified on real `--tty`** (dev VM, virtio-gpu KMS at
1600x1000, release binary, **re-run in full at `41b7a50`** -- both review
rounds changed the serve loop, so every earlier run's cache key is stale and
every figure in this entry is from the last one): `/proc/<pid>/wchan` read
`do_epoll_wait` throughout -- never `unix_stream_read_generic`, the
observable this item was diagnosed by. While one client held
`{"type":"vers` for 20 seconds: three round-trips served (361/189/196us),
`flexwm msg windows`, a full 33,476-byte screenshot and a `wait-idle`
(`waited_ms: 203`) all answered, and the compositor burned 0 jiffies over
the whole window. Four clients holding partial lines simultaneously were
each answered their own reply. A client that wrote 14,136 bytes of requests
and read none (back-pressure engaged -- its own writes stopped being
accepted) delayed nobody, and when it finally read: 37,200 bytes, 744 reply
lines, **0 damaged**, exactly the 744 expected, so an interleaved write
never corrupted the framing. The flood-versus-round-trip figures above hold
here too (re-measured at this SHA: quiet p50 125us/p99 244us over 23,180
round-trips, flooded p50 353us/p90 384us/p99 553us over 17,943 while the
flooder pushed 390.6 MB in 8s). Idle CPU 0 jiffies over 10s both before and
after everything, so neither the interest switching nor the read budget
introduced a spin; one WARN in the whole log, smithay's own
`Failed to destroy old mode property blob` at modeset. The 1 MiB request cap
re-verified against the same release binary over a real socket: the flood
dies at 1,114,112 bytes, the refusal arrives intact first, RSS stays at
10,668 kB (VmPeak 21,520 kB -- item 9's figures), the compositor survives
and a fresh connection still works (195us).

**Benchmarked** (item 9's method: release, 50,000 `version` round-trips
over one connection, compositor jiffies plus us/round-trip) **with the run
order balanced**, because position within a rep turned out to be worth more
than the change being measured. Re-run at `41b7a50`: 20 reps per side, 10
with the pre-change binary first and 10 with it second. Pooled medians:
**126.03us/71 jiffies after versus 127.57us/71 before**. Three of the 40
runs came in under 110us (79.7, 81.5, 88.3) -- one on the pre-change side,
two on the other, which is what makes them VM scheduling noise rather than
a property of either binary, and why this is reported as medians. No
measurable difference on the uncontended round-trip path, which is the path
the fairness budget adds a comparison to; the same answer the first round's
28-reps-per-side run gave, with the sign flipped.

`scripts/smoke-test.sh` green under `--headless` and `--nested` (under
`cage`), 175/175 tests, clippy and fmt clean. The whole compositor is
`#[cfg(target_os = "linux")]`, so none of this can be verified from the Mac
host -- every figure here is from the dev VM guest.
