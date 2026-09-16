---
title: "No cap on concurrent IPC connections, and a half-closed client leaks one (MEDIUM) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No cap on concurrent IPC connections, and a half-closed client leaks one (MEDIUM) — RESOLVED

This entry was filed as "Screenshot capture runs synchronously on the sole
event-loop thread with no rate limit", and carried three concerns under that
one title. Two of them are closed here and the title now names them; the
third is still open and has been re-filed on its own so it is not lost with
this file:
[screenshot capture and encode on the event-loop thread](../ipc/screenshot-encode-on-event-loop.md).

## What it said (the two concerns closed here)

The rate limit was DONE as item 9 (one capture per connection per 16ms
frame, refused rather than delayed). Still open, and deliberately out of
scope there: the IPC accept loop had no cap on concurrent connections, so
the per-connection limit was bypassable by reconnecting for every capture,
and nothing bounded how many connections one client could hold open.

One more lifecycle case belonged here, found by `flexwm-reviewer` while
reviewing item 10: a client that half-closes (`shutdown(SHUT_WR)`) and then
never reads pins its connection slot and two fds for good. A half-close
raises `EPOLLIN`/`EPOLLRDHUP`, not `EPOLLHUP`, and a connection with a queue
is registered for writability only, so nothing wakes it again. Deliberately
not fixed in item 10: registering for reads there would spin the loop at
full speed on an end-of-stream that can never be acted on (the queue cannot
drain), which is worse. Strictly better than the pre-item-10 behavior, where
that same case froze the whole compositor — but still a live resource leak a
connection cap would need to account for.

## Resolution (2026-09-16)

**A cap on concurrent connections** (`ipc/slots.rs`). 64 at once across every
client, claimed in `accept` and released by dropping the claim, so every path
a connection can leave by — closed, evicted, the loop torn down — gives the
slot back without having to remember to. Past the cap a connection is refused
rather than queued, and told why: the compositor writes one `Response::Error`
line naming the limit and closes, which `flexwm msg` prints as an ordinary
error. Refusals are logged at `debug`.

Sized the way the compositor's other bounds are (`wl_shm` pool caps,
`MAX_TOKENS`, `input/interaction.rs`'s `CAPACITY`): far above any real
workload — a session runs a bar, a notifier and an agent or two, each holding
one connection, and a one-shot `flexwm msg` holds one for a millisecond — so
a session holding 64 at once has a client in a loop, not a busy desktop.

A `wait-idle` hand-off moves its slot into the waiter rather than releasing
it. Without that the cap would be decorative: a client could park every
connection it opened in a `wait-idle` whose `quiet_ms` never comes due, free
the slot on the way, and hold an unbounded number of fds in `pending_idle`.

**A write-stall deadline** (`connection.rs`, `WRITE_STALL_TIMEOUT`, 10s). The
half-close case is not fixed by registering for reads — the reason item 10
rejected that stands — so it is fixed on a deadline instead: a calloop
`Timer` composed into `ConnectionSource`, armed exactly while the connection
is registered for writability (i.e. exactly while it has a queue its peer has
not taken), comparing how many bytes have left the queue against how many had
left it a window ago. Not one byte in a whole window means the peer is not
reading, and the connection is dropped. It is a bound on *progress*, not on
total time — exactly how `PendingIdle::push` already gives up on a
`wait-idle` answer — so a client slowly draining a multi-megabyte screenshot
is never given up on.

Counted in bytes that *left* rather than in bytes still queued, which is not
the same test and was the first version of this (caught bug-bashing the
diff): a queue the same size a window later may have drained and refilled in
between, which is what a client pipelining a batch and reading the answers as
it goes looks like from the compositor's side. Watching the queue's depth
would have dropped that connection mid-batch for reading slower than it was
being answered.
An idle connection (nothing queued) is not armed at all and costs no wakeups:
measured 0 jiffies over 10s with a live idle client, before and after.

Shown on hardware rather than argued (dev VM, release build, real socket,
`RUST_LOG=flexwm=debug`): a client that writes 228,736 bytes of requests,
half-closes and never reads takes the compositor from 11 to 13 open fds. On
the pre-change binary it is still at 13 fds thirty seconds later. On this one
it is back to 11 within fifteen, with
`dropped an ipc connection whose peer stopped reading its reply pending=7650
stall_ms=10000` in the log. The cap, the same way: 64 connections all
answered, the 65th told `refused: flexwm serves at most 64 ipc connections at
once...` and closed, a connection that was let in still answering, and the
table handing slots out again once they closed.

**Benchmarked** on the round-trip path it touches (item 9/10's method:
release, 50,000 `version` round-trips over one connection, balanced run
order, 6 reps a side). Medians **130.13us/72 jiffies after versus
129.96us/71.5 before** — 0.13%, against a spread of 1.8us within the
"before" side alone, so no measurable difference. That is what the shape
predicts: a connection with nothing queued adds one `Timer::process_events`
call per wakeup against an unregistered timer, one `u64` add per socket
write, and one `Cell` increment per accept.
