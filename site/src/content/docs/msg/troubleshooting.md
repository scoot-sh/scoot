---
title: IPC troubleshooting
description: "What the socket refuses, the resource bounds behind it, and the symptom for each."
---

The socket says no in known ways: the nine bounds an agent can actually hit, the resource limits behind them, and what to do about each.

## What the socket refuses

Nine bounds an agent can actually hit. The first seven are refusals with a
reason — an ordinary `error` response — rather than a silent drop or a delay.
The last two can't be: one drops a peer that is by definition not reading its
socket, and the other shortens a wait rather than refusing it.

- **One request line may be at most 1 MiB.** Past that the connection is told
  so and closed; there is no resynchronizing mid-line.
- **`type` text is limited to 16,384 characters per request.** Each character
  becomes key events typed synchronously on the thread that serves every
  other client, so a megabyte of text would stall the whole compositor for
  seconds. Split the text across several `type` requests. A shell command
  line's worth costs under a millisecond and never notices this. Counted in
  characters, not bytes; even the longest encodings land far under the 1 MiB
  line limit, so this cap always fires first.
- **One screenshot per connection per 16 ms frame.** A capture costs a render
  and framebuffer read-back on the thread that serves every other client, so
  a second one inside the same frame is refused rather than queued. Retry
  after a frame. Note the reply order for that pair: the refusal is answered
  immediately, while the capture it follows is still encoding — so the
  *second* request's reply arrives *first*. A client pipelining screenshots
  matches replies by content, not by position.
- **The `--tty` GPU scanout tier can refuse a capture with a retry.**
  (`--renderer gles` in a `gpu-scanout` build only.) It never serves the
  screen as it was before a client's buffer went on screen directly.
  There the capture reads the buffer the last composited frame landed in, so
  it is refused as `nothing has been scanned out yet` before the first
  frame, and as `the current frame is held for direct scanout; retry once a
  composite frame lands` when the screen is showing a client's buffer
  directly and the composite frame the capture forces could not be drawn
  or recorded (the session is VT-switched away, the forced render failed,
  or its buffer could not be exported). Normally the capture forces that
  composite frame itself and succeeds. The screen shows a client's buffer
  directly while a fullscreen window covers the output, its buffer is a
  dma-buf the display can import, and no capture stream is running on that
  output (see [tty.md](../scoot/backends.md)), so the second refusal is reachable in
  normal use --
  observed on the dev VM for a screenshot taken while VT-switched away,
  with the next one after the switch back served. Treat both as retryable.
  A screenshot of a direct frame also takes a little longer for the
  composite frame it forces: a median 3-7 ms more on the dev VM, whether
  polled once a second or ten times a second.
- **One capture in flight per connection.** The PNG encode runs on a worker
  thread, so other connections are answered while it runs — but the capture's
  own reply still has to go out before any later reply on that same
  connection, or a client reading replies in request order sees them swap.
  Any other request arriving on a connection with a capture in flight is
  refused with a retry rather than answered out of order. `scoot msg` sends
  one request per connection and never meets this; nor does a capture refuse
  one on *another* connection. A capture whose earlier replies are still
  going out is refused the same way — retry once the queue has drained.
- **Four captures in flight at once, across every client.** Each holds a full
  frame of raw pixels on its way through the worker, so past four the next
  capture is refused with a retry rather than queued without bound. The
  encode does not block the event loop; the render and read-back still cost
  the loop a moment per capture, so under N concurrent capturers a bystander
  waits longer than one capture's share.
- **At most 64 connections at once**, across every client. A 65th is refused
  with a message naming the limit and closed immediately, not queued behind
  the others. This is what keeps the per-connection bounds meaningful.
- **A newcomer under file-descriptor pressure is shed, each channel on its
  own line.** While fewer than 128 fds stand free process-wide, a new
  Wayland connection gets an immediate EOF (there is no protocol channel
  for a reason) — but a new IPC connection is still admitted until fewer
  than 16 stand free. Past that second line it is refused with `refused:
  scoot is under file-descriptor pressure (fewer than 16 fds free); retry
  in a moment -- this connection cost nothing, and pressure lifts as soon
  as whoever is holding fds lets go` and closed immediately — no slot
  taken, living connections untouched. Retry in a moment: pressure lifts
  as soon as whoever is holding fds lets go, and ordinary use (an idle
  compositor holds 14 fds, a `foot` window 17) never comes near either
  line. That 16 is headroom for what one served request transiently opens
  past its socket, not for 64 fd-heavy requests landing together at the
  boundary — there a dial can still see EOF, the same as a literally full
  table.
- **A connection whose peer stops reading is dropped**, ten to twenty seconds
  after the last byte it took (the check runs on a deadline of its own).
  Nothing is sent when this happens — there is nobody reading to send it to;
  the connection simply closes. Replies that don't fit in the socket are
  queued and pushed out as the client reads; a client that takes no bytes at
  all for that long — the classic case being one that sends a request, does
  `shutdown(SHUT_WR)` and then never reads the answer — is treated as gone.
  Reading *slowly* is fine and is never given up on: the clock runs from the
  last byte that actually went out, not from when the reply was queued, so
  draining a multi-megabyte screenshot over a minute costs nothing.
- **`wait-idle` waits at most 60 seconds**, whatever `--timeout-ms` asks for.
  A longer request isn't refused, it's shortened: the answer comes back at
  the minute mark at the latest. A waiting `wait-idle` keeps its connection
  (and one of the 64 slots) for as long as it waits, and — uniquely on this
  socket — cannot notice its client dying while it waits. The default is 5
  seconds and the request is meant for hundreds of milliseconds. A capture in
  flight neither extends nor shortens a wait.

## Resource bounds

Sizes a client or a config supplies are bounded, so one greedy or buggy
client cannot exhaust the compositor for the others. Ordinary use sits orders
of magnitude below all of these; they matter if you are writing a client that
allocates in a loop.

| Bound | Value | What happens past it |
| --- | --- | --- |
| `wl_shm` pool size | 512 MiB each | Protocol error on `create_pool`. Four full-screen 8K frames' worth. |
| Live `wl_shm` pools per client | 128 | Protocol error on the excess `create_pool`. Bounds live pool objects and the address-space envelope — *not* fds or mappings, since a buffer outlives its pool and retains both. |
| Live `wl_buffer`s per client | 512 | Protocol error on the creating object, whatever created it (pool, dmabuf, single-pixel). Each surviving shm or dmabuf buffer is what retains a compositor fd and, for shm, its mapping; single-pixel buffers retain neither but are counted uniformly, because the hook can't observe buffer kind. |
| Process-wide free fds | 128 for Wayland, 16 for IPC | While fewer than 128 stand free, a new Wayland connection gets an immediate EOF (there is no protocol channel for a reason); a new IPC connection is still admitted until fewer than 16 stand free, past which it is refused with a message. A client already holding past 128 live buffers or 64 live pools is refused its next creation with the same protocol error, so a client under those graces is never refused for another client's greed. |
| Manager/list binds per client | 8 | Across the workspace, both window-list and display-management globals combined. The ninth bind is closed with `finished` (plus the `done` batching requires) and announced nothing — a greedy client costs itself its ninth subscription, never another client's. |
| Unredeemed activation tokens | 64 | Across all clients, expired ones swept first. A spawn past a full table simply gets no token. |
| Live capture frame objects per client | 16 | The protocol's own `duplicate_frame` error, which disconnects the client that overflowed. |

An imported dmabuf's mapping is the one thing the buffer count does not
bound, because it lives in the renderer's cache and outlives the `wl_buffer`
that carried it; it is released instead from the same buffer-destruction
hook, immediately and without waiting for a frame.

Config-supplied sizes are bounded the same way: a client's declared minimum
window size can't exceed the largest output's usable area on each axis, and
`gap` and `cursor_size` each have an upper bound as well as a lower one — see
[configuration.md](../scoot/configure.md).

