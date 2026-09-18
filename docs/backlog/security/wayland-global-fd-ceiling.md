---
title: "A global fd/buffer ceiling across Wayland connections."
status: "open"
area: "security"
priority: "low"
blocked: null
---

# A global fd/buffer ceiling across Wayland connections.

Split out of [the Wayland connection-cap
verdict](../resolved/wayland-connection-cap-done.md) (2026-09-18), which
fixed the compositor-killing half (an `EMFILE` on the Wayland listener used
to exit the whole process; now it sheds) and closed the connection-count
half as an accepted tradeoff (any usable count admits the two greedy
connections that fill the 1024-fd table, so a count denies shells while
stopping nothing).

What remains open is the residual that fix accepts: per-connection bounds
(512 buffers, 128 pools, 8 binds, 16 capture frames -- all test-pinned, see
the verdict) still multiply across connections, so two greedy connections
can hold ~1039 fds and deny pools, sockets and mmaps to every innocent
client until pressure lifts. The compositor survives; who notices changed.

The option the verdict decided against *for now* is a ceiling shared across
connections -- a global live-buffer total, a global pool total, or both --
sized so the worst case stays under the fd table with headroom. The reason
is the refusal form, not the accounting: today's per-connection guards kill
only the offender with a protocol error, while a shared ceiling fires on an
innocent client for another's greed. That is harsher than the IPC cap's
refused-with-reason (which at least names the limit), and it is the shape
`bind_budget.rs` deliberately refused to build ("a greedy client must not
deny a well-behaved one"). Landing a ceiling means designing that refusal
first -- silent ignore is out (it leaves the uninitialized object that
panics the compositor), killing a victim needs a justification this project
has not written, and waiting/failing the creation has no protocol channel
on some of these interfaces.

Revisit if a real workload ever wedges innocents this way. The arithmetic
to re-derive it against is in the verdict; the per-connection numbers have
not moved since.
