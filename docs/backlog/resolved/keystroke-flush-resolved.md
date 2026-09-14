---
title: "A real keystroke (libinput, or nested host-forwarded input) can sit unflushed to the client for seconds on a quiet screen (MEDIUM) \u2014 DONE as item 11"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A real keystroke (libinput, or nested host-forwarded input) can sit unflushed to the client for seconds on a quiet screen (MEDIUM) — DONE as item 11

~~A real keystroke (libinput, or nested host-forwarded input) can sit
unflushed to the client for seconds on a quiet screen (MEDIUM)~~ — DONE as
item 11, PR #17, fixed exactly as suggested here (the one-line
post-dispatch flush) and re-verified on real `--tty` *and* `--nested`
hardware both before and after. Two notes on this entry's own framing: the
"matching Smithay's own `anvil` pattern" half is true in substance but not in
form -- anvil hand-rolls its dispatch loop rather than using
`EventLoop::run`'s callback (see item 11) -- and the closing suggestion that
item 10's `ipc/connection.rs` invariant "would likely be simplified too" was
deliberately *not* acted on: it is well-tested, three review rounds old, and
redundant-but-earlier rather than wasteful, since a flush with nothing queued
issues no syscall. Original diagnosis, left as written: found by
`flexwm-reviewer` while reviewing item 10, unrelated to and not caused by
that PR (confirmed identical on `main` before it). Reproduced twice on
real `--tty` with a genuine `/dev/uinput`-injected key: after the press,
total IPC silence for 6s, then the very next screenshot's own end-of-
wakeup flush is what finally delivers the character to the client — one
screenshot shows the key hasn't visibly arrived yet (pixel-identical to
before the press), the next one 1.5s later shows it has, with no new input
or IPC traffic in between other than the screenshots themselves. Chain:
`tty/mod.rs`'s `libinput_event` queues the client's `wl_keyboard` message,
but nothing in the keyboard path calls `request_render()` (only pointer
motion does, and only under `--tty`) or flushes on its own; `render()`
early-returns before its own flush when `!needs_render`; the frame timer
drops itself when idle. So on an otherwise-quiet screen, a keystroke is
invisible until something else (mouse motion, another IPC request)
happens to trigger a flush. Item 10's fix closed this exact shape for the
IPC-injected-input path specifically; this is the same root cause on the
real-hardware-input path, which item 10 doesn't touch. The idiomatic fix:
a single `let _ = state.display_handle.flush_clients();` in the
post-dispatch callback of `event_loop.run` (`compositor/mod.rs`, currently
a no-op `|_| {}`) — matching Smithay's own `anvil` reference compositor's
pattern — would make "every dispatched event eventually gets flushed"
structural instead of a per-call-site discipline to remember, and would
likely let item 10's own hand-maintained "every exit reaches the flush"
invariant in `ipc/connection.rs` be simplified too.
