---
title: "The IPC connection loop does blocking I/O, one line per readiness event (HIGH; pre-existing) \u2014 DONE as item 10."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The IPC connection loop does blocking I/O, one line per readiness event (HIGH; pre-existing) — DONE as item 10.

~~The IPC connection loop does blocking I/O, one line per readiness event
(HIGH; pre-existing)~~ — DONE as item 10. The diagnosis below is left
as-written because it is what the fix was built against; item 10 records what
shipped, which part of this diagnosis turned out slightly off, and what it
deliberately left alone. Found while bug-bashing item 9;
every symptom below verified identical on the pre-item-9 binary, so none is
a regression, and all were explicitly left unfixed there. Raised from MEDIUM
to HIGH by `flexwm-reviewer`'s pass on PR #14, which reproduced symptom (a)
more precisely than item 9's own bug bash did: a half-written request line
does not merely hang that one connection, it freezes the *entire* event loop
— `/proc/<pid>/wchan` reads `unix_stream_read_generic` and CPU time goes
completely flat, so no frame tick, no wayland dispatch and no input
processing happens for any client, for as long as one connection holds a
partial line. It needs no malice at all: a client killed mid-paste, or any
agent that writes a request in chunks, does it. That makes it the first
thing to pick up from this backlog rather than one more unordered entry.

`accept()` puts the connection into *blocking* mode and `Connection::step`
reads exactly one line per readiness event, which causes three distinct
symptoms with one root cause:
(a) a client that writes `{"type":"vers` and holds the connection open
parks the single event-loop thread inside `fill_buf` — every other IPC
client, wayland dispatch and input stop until it sends a newline or
disconnects (confirmed: a second client's perfectly valid request went
unanswered for a 20s read timeout, then was served in 86us the moment the
stalled client went away). This is a local hang DoS that needs no
malice — a crashed agent mid-write does it — and it is a *worse* version of
the finding item 9's line cap closed, since it costs the attacker one byte
instead of a megabyte.
(b) two requests in one `write()` get one reply: the `BufReader` drains
both from the socket, the level-triggered source sees nothing more to read,
and the second line sits in the buffer unanswered until unrelated traffic
wakes the connection. Nothing in-tree pipelines (both `flexwm msg` and
`flexwm_ipc::Client` are strict request/response), so this is latent, but it
is a protocol surprise for any agent that batches.
(c) `reply()` writes blocking too, so a client that keeps sending requests
and never reads the answers fills its own receive buffer and deadlocks the
compositor inside `write_all` (confirmed: the probe's own `write_all` blocked
in turn, before it could even open a second connection to test with, and the
compositor was answering again 61us after that client was killed). Same
one-byte-of-effort shape as (a), from the other direction.
Fix direction, one change for all three: leave the stream non-blocking, have
the bounded reader return "incomplete, keep what you have" on `WouldBlock`
(clearing the buffer only once a line has been consumed, so the 1 MiB cap
still applies across however many reads a line takes), loop `step` over
every complete line already buffered before returning to the event loop, and
give each connection an outbound buffer plus `Interest::WRITE` when a reply
cannot go out in one go — with a cap on that buffer, since it is the same
unbounded-growth shape item 9 just closed on the read side. That restructures
the connection loop and wants its own test matrix (partial lines interleaved
with `WaitIdle`'s hand-off and the screenshot limiter), which is why it is
its own item rather than a rider on item 9.
