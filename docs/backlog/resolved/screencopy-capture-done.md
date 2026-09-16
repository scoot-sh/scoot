---
title: "Screen capture for clients (`ext-image-copy-capture-v1`) for shell thumbnails and previews — RESOLVED (output half)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Screen capture for clients (`ext-image-copy-capture-v1`) for shell thumbnails and previews — RESOLVED (output half).

## The entry as filed

> Both shell probes hit this (DMS gap 6, Noctalia gap 6, 2026-09-14):
> launcher window thumbnails and workspace-overview live previews
> (`ScreencopyView`) have nothing to consume — neither global is
> advertised. Previously bundled in
> `docs/backlog/protocols/protocol-gaps-niche.md`.
>
> This is purely a shell-client gap, not an agent gap: flexwm's own
> screenshots for computer-use automation go over the privileged IPC
> socket (`flexwm msg screenshot`, owner-only, rate-limited), which stays
> regardless. The standard-protocol path is for third-party tools
> (`grim`, `wf-recorder`, shell thumbnails, conferencing screen-share)
> that will never speak flexwm IPC.
>
> Per the standing rule, prefer the standard protocol: evaluate
> `ext-image-copy-capture-v1` (with `ext-image-capture-source-v1`)
> first, `wlr-screencopy-unstable-v1` for legacy clients second. Note
> the privacy interaction with session lock: captures taken while
> `ext-session-lock-v1` holds the session must see only the lock
> framebuffer, never the windows behind it — the same guarantee
> `flexwm msg screenshot` already gives. Rough size: M.

## What shipped

`crates/flexwm/src/compositor/screencopy.rs`, against Smithay's
`wayland::image_copy_capture` and `wayland::image_capture_source` at the
pinned rev (`0ff00983`) — unlike the three protocol items before it, this one
had a maintained upstream implementation, so flexwm writes handlers rather
than wire format.

Two globals:

- `ext_image_copy_capture_manager_v1` (version 1)
- `ext_output_image_capture_source_manager_v1` (version 1)

Neither filtered by client, same trust model as every other global here (no
security-context support, so an allow-list would be theatre) — called out
explicitly in `README.md` because this is the protocol where "any process that
can reach the socket can read your screen" is most worth stating plainly.

### The `ext-` protocol, and no wlr protocol alongside it

`CLAUDE.md`'s standing rule is the `ext-` successor where one exists. PR #50
made the opposite call for the window list after measuring that the client
that mattered did not speak it, so the same measurement was taken here first
(dev VM nix store, 2026-09-16):

- `grim` 1.5.0: `ext_image_copy_capture_*` and `ext_image_capture_source_v1`
  symbols present; **no** `zwlr_screencopy_manager_v1`.
- `quickshell` 0.3.1 (the build DMS and Noctalia run on):
  `ext_image_copy_capture_manager_v1`,
  `ext_output_image_capture_source_manager_v1`,
  `ext_foreign_toplevel_image_capture_source_manager_v1` **and**
  `zwlr_screencopy_manager_v1` — i.e. it speaks the `ext-` protocol and keeps
  the older one as a fallback.

So nothing measured needs `wlr-screencopy-unstable-v1`, and it is not
implemented.

### Output capture only

The toplevel source manager is deliberately not advertised — not advertised
and refused, so a client takes its fallback path immediately rather than
discovering a `stopped` at runtime. Capturing one window in isolation needs a
second render target per session, a constraint-refresh path driven by window
resizes, mid-session teardown when the window closes, and a stated answer for
a locked session; that is this module again, so it is
[its own item](../protocols/screencopy-toplevel-capture.md) — the same split
PR #47 and PR #49 made.

### How a capture is served

A `capture` request does not copy pixels there and then. The frame is parked
on its session and served from `State::service_captures`, which runs on the
frame tick immediately after `State::render`:

- it bounds what a client can ask for — at most one capture per session per
  frame tick, the same reason `ipc.rs` spaces a connection's screenshots by
  `FRAME_INTERVAL`;
- it is what the protocol describes ("unless this is the first successful
  captured frame performed in this session, the compositor may wait an
  indefinite amount of time for the source content to change"), so a
  session's first frame is served on the next tick whatever the screen is
  doing and every later one waits for `State::frame_serial` to move — a
  static desktop costs a `u64` comparison per tick and nothing else;
- it keeps the copy out of Wayland request dispatch, so nothing re-enters
  `render()` from inside a client's request.

The framebuffer read-back happens **once per tick**, shared by every session
that is due — `PixmanRenderer::copy_framebuffer` allocates and fills a fresh
image per call, so doing it per session would cost an extra full copy of the
screen for each client watching.

### The session-lock guarantee

Inherited, not re-derived: `render()` decides once per frame whether it is
drawing the lock screen or the desktop, and this reads back that same
framebuffer through the same `bind`/`copy_framebuffer`/`map_texture` sequence
`screenshot.rs` uses. The one thing reuse does not cover is the window between
a lock being accepted and its first blanked frame reaching the framebuffer
(`is_locked()` already true, desktop pixels still there) — reachable when a
render fails, since `render()` clears `needs_render` either way. Captures are
not served at all while a lock is pending (`SessionLock::awaiting_blank`);
the frame stays parked, which the protocol's "may wait indefinitely" allows.

### Bugs found and fixed while building it

- **Smithay leaks every session a client ever creates.** `SessionData::
  destroyed` calls the compositor's `session_destroyed` but does not remove
  the `SessionRef` from `ImageCopyCaptureState::sessions`; only `cleanup()`
  does, and nothing upstream calls it. The `capture` request then walks that
  list linearly. `session_destroyed` calls `cleanup()` here, and
  `a_destroyed_session_leaves_the_compositors_list` asserts on *both* lists so
  a version that swept only flexwm's own would fail.
- **Smithay never raises `duplicate_frame`.** The protocol allows at most one
  frame object per session at a time; the pinned rev pushes every
  `create_frame` onto an unbounded list. The parked-frame slot is the bound:
  a second outstanding capture is failed rather than queued.

### Known deviation, recorded rather than discovered

`paint_cursors` is accepted and has no effect. Under `--headless`/`--nested`
nothing draws a cursor, so a capture never contains one; under `--tty` the
cursor is an element in the one framebuffer a capture is read out of, so a
capture always contains it. Honouring the flag means a second full-output
render with the cursor dropped. `flexwm msg screenshot` has the same property.

### What is still unbounded

How many sessions one client may hold — N sessions with N parked frames cost
N full-screen copies on a tick the screen changes. Filed as
[its own item](../protocols/screencopy-session-cap.md) with the reasoning for
why it was not simply capped (the same per-frame work N mapped surfaces
already demand; and a *global* cap would let one client deny another, which
Smithay's handler API gives no per-client alternative to).

## Tests

`crates/flexwm/src/compositor/screencopy/tests.rs`, eleven real-client tests.
Every one drives a real `wayland-client` connection, allocates a real `wl_shm`
buffer over a real memfd, and **reads that memfd back** to compare the
client's own bytes with the framebuffer — the claim under test is about what
landed in a client's buffer, which a compositor-side assertion cannot make.

- constraints name the framebuffer size, both formats, opaque first, one
  `done`;
- a capture equals the framebuffer byte for byte, and contains the window that
  was on screen;
- an `Xrgb8888` capture is opaque everywhere over a deliberately translucent
  background, and differs from the framebuffer in nothing but the fourth byte;
- a capture while locked carries the lock surface and no pixel of the window
  behind it;
- a capture parked *before* a lock and served *after* it still carries no
  desktop pixel — the race the guarantee actually has to survive;
- a later capture of an unchanged screen is not served, and is served the
  moment something changes, carrying the new frame;
- an undersized buffer is `failed(buffer_constraints)`;
- a resized output re-advertises `buffer_size` with its own `done`;
- a second outstanding capture on one session is refused;
- a destroyed session leaves both session lists;
- a client disconnecting with a capture outstanding leaves both lists correct
  and a second client is still served.

## Verification

See the evidence block appended below.
