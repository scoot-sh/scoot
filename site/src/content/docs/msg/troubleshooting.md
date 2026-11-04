---
title: IPC troubleshooting
description: "What the socket refuses, the resource bounds behind it, and the symptom for each."
---

The socket says no in known ways: the ten bounds an agent can hit and what to do about each.

## What the socket refuses

The first eight are refusals with a reason — an ordinary `error` response.
The last two can't be: one drops a peer that is not reading,
and the other shortens a wait rather than refusing it.

- **One request line may be at most 1 MiB.** Past that the connection is told
  so and closed; there is no resynchronizing mid-line.
- **`type` text is limited to 16,384 characters per request.** Split longer text
  across several `type` requests. Counted in characters, not bytes, so
  this cap always fires before the 1 MiB line limit.
- **One screenshot per connection per 16 ms frame.** A second one inside
  the same frame is refused rather than queued. Retry
  after a frame. The refusal is answered
  immediately while the capture it follows is still encoding, so the
  *second* request's reply arrives *first*. A client pipelining screenshots
  matches replies by content, not by position.
- **The `--tty` GPU scanout tier can refuse a capture with a retry.**
  (`--renderer gpu` in a `gpu-scanout` build only.) There the capture reads
  the buffer the last composited frame landed in, so
  it is refused as `nothing has been scanned out yet` before the first
  frame, and as `the current frame is held for direct scanout; retry once a
  composite frame lands` when the screen is showing a client's buffer
  directly and the composite frame the capture forces could not be drawn
  or recorded (the session is VT-switched away, the forced render failed,
  or its buffer could not be exported). Normally the capture forces that
  composite frame itself and succeeds. The screen shows a client's buffer
  directly while a fullscreen window covers the output, its buffer is a
  dma-buf the display can import, and no capture stream is running on that
  output (see [tty.md](../scoot/backends.md)), so the second refusal is
  reachable in normal use. Treat both as retryable.
  A screenshot of a direct frame takes a little longer for the
  composite frame it forces: a median 3-7 ms more on the dev VM.
- **One capture in flight per connection.** Any other request arriving on a connection with a capture in flight is
  refused with a retry rather than answered out of order. `scoot msg` sends
  one request per connection and never meets this, and a capture does not
  refuse a request on *another* connection. A capture whose earlier replies are still
  going out is refused the same way — retry once the queue has drained.
- **Four captures in flight at once, across every client.** Past four, the next
  capture is refused with a retry rather than queued. The
  encode does not block the event loop; the render and read-back still cost
  the loop a moment per capture.
- **At most 64 connections at once**, across every client. A 65th is refused
  with a message naming the limit and closed immediately.
- **A newcomer under file-descriptor pressure is shed, each channel on its
  own line.** While fewer than 128 fds stand free process-wide, a new
  Wayland connection gets an immediate EOF (there is no protocol channel
  for a reason), but a new IPC connection is still admitted until fewer
  than 16 stand free. Past that second line it is refused with `refused:
  scoot is under file-descriptor pressure (fewer than 16 fds free); retry
  in a moment -- this connection cost nothing, and pressure lifts as soon
  as whoever is holding fds lets go` and closed immediately, taking no slot
  and leaving living connections untouched. Ordinary use (an idle
  compositor holds 14 fds, a `foot` window 17) never comes near either
  line. At the boundary, 64 fd-heavy requests landing together can still
  see EOF, like a full table.
- **A connection whose peer stops reading is dropped**, ten to twenty seconds
  after the last byte it took. Nothing is sent when this happens; the connection simply closes. Replies that don't fit in the socket are
  queued and pushed out as the client reads; a client that takes no bytes at
  all for that long (for example one that sends a request, does
  `shutdown(SHUT_WR)` and never reads the answer) is treated as gone.
  Reading *slowly* is fine: the clock runs from the
  last byte that actually went out, so
  draining a multi-megabyte screenshot over a minute costs nothing.
- **`wait-idle` waits at most 60 seconds**, whatever `--timeout-ms` asks for.
  A longer request isn't refused, it's shortened: the answer comes back at
  the minute mark at the latest. A waiting `wait-idle` keeps its connection
  (and one of the 64 slots) while it waits, and, uniquely on this
  socket, cannot notice its client dying while it waits. A capture in
  flight neither extends nor shortens a wait.

## Resource bounds

Sizes a client or a config supplies are bounded. Ordinary use sits far below all of these.

| Bound | Value | What happens past it |
| --- | --- | --- |
| `wl_shm` pool size | 512 MiB each | Protocol error on `create_pool`. |
| Live `wl_shm` pools per client | 128 | Protocol error on the excess `create_pool`. Bounds live pool objects, not fds or mappings: a buffer outlives its pool and retains both. |
| Live `wl_buffer`s per client | 512 | Protocol error on the creating object, whatever created it (pool, dmabuf, single-pixel). Each surviving shm or dmabuf buffer retains a compositor fd and, for shm, its mapping. Single-pixel buffers retain neither but are counted the same. |
| Process-wide free fds | 128 for Wayland, 16 for IPC | While fewer than 128 stand free, a new Wayland connection gets an immediate EOF (there is no protocol channel for a reason); a new IPC connection is still admitted until fewer than 16 stand free, past which it is refused with a message. A client already holding past 128 live buffers or 64 live pools is refused its next creation with the same protocol error. |
| Manager/list binds per client | 8 | Across the workspace, both window-list and display-management globals combined. The ninth bind is closed with `finished` (plus the `done` batching requires) and announced nothing. |
| Unredeemed activation tokens | 64 | Across all clients, expired ones swept first. A spawn past a full table simply gets no token. |
| Live capture frame objects per client | 16 | The protocol's own `duplicate_frame` error, which disconnects the client that overflowed. |

An imported dmabuf's mapping is not bounded by the buffer count. It is
released as soon as the buffer is destroyed, without waiting for a frame.

Config-supplied sizes are bounded the same way: a client's declared minimum
window size can't exceed the largest output's usable area on each axis, and
`gap` and `cursor_size` each have an upper bound as well as a lower one — see
[configuration.md](../scoot/configure.md).

