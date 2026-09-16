---
title: "Screen capture, toplevel half: `ext_foreign_toplevel_image_capture_source_manager_v1`."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Screen capture, toplevel half: `ext_foreign_toplevel_image_capture_source_manager_v1`.

Split out of
[`screencopy-capture-done.md`](../resolved/screencopy-capture-done.md) when the
output half shipped, the same way PR #49 split output-management
reconfiguration and PR #47 split the wlr foreign-toplevel list — so what
shipped and what did not are two separate records rather than one half-true
one.

flexwm now advertises `ext_image_copy_capture_manager_v1` and
`ext_output_image_capture_source_manager_v1`, so a client can capture the
**output**. It does **not** advertise
`ext_foreign_toplevel_image_capture_source_manager_v1`, so a client cannot
capture one **window** on its own. That is the half a launcher's per-window
thumbnails need (DMS `TileItem.qml`'s `ScreencopyView`, Noctalia's
equivalent); the workspace-overview preview is the output half and works
today.

Deliberately not advertised-and-refused: a client that can create a source
and is then told `stopped` has to discover the refusal at runtime, where one
that never sees the global takes its own fallback path immediately.

## Why it is a separate item rather than the rest of one PR

Output capture reads a region back out of the one framebuffer the compositor
already drew. A toplevel capture cannot: the protocol asks for *that window's*
content, not the part of the screen it happens to occupy (which may be
obscured, clipped by the output edge, or scrolled off it entirely in a
scrolling-column layout). So it needs, in rough order of size:

- **A second render target per session.** The window's own surface tree
  rendered into an isolated buffer — `Window` is `AsRenderElements`, so an
  `OutputDamageTracker` sized to the window over a per-session offscreen
  pixman image is the shape, but it is a render target flexwm does not have
  today, with its own allocation and resize lifecycle.
- **A constraint-refresh path of its own.** `buffer_size` is the window's
  size, and a window resizes on every layout change — far more often than the
  output's mode changes, which is the only refresh the output half needed
  (`State::refresh_capture_constraints`).
- **Mid-session teardown.** A window closing has to `stop` every session
  bound to it. `shell.rs`'s `remove_window` is the choke point, alongside the
  two foreign-toplevel lists it already drives.
- **A stated answer for a locked session.** There is no "lock version of a
  window" the way there is a lock *screen* for the output, so the natural
  answer is to refuse toplevel capture outright while
  `ext-session-lock-v1` holds the session — mirroring how
  `foreign_toplevel_management.rs` refuses `activate`/`close` while locked
  rather than faking a safe one. That is a decision to make and document, not
  something to let fall out of the code.
- **A bridge from `ForeignToplevelHandle` to `WindowId`.**
  `ToplevelCaptureSourceHandler::toplevel_source_created` is handed Smithay's
  `ForeignToplevelHandle`; `foreign_toplevel.rs` keys its own map the other
  way round (`WindowId -> handle`), and the identifier
  (`<generation>-<window id>`) is the only thing that connects them.

## One measurement to take first

`ext_foreign_toplevel_image_capture_source_manager_v1.create_source` takes an
**`ext_foreign_toplevel_handle_v1`**, i.e. a handle from
`ext-foreign-toplevel-list-v1`. PR #50 measured that stock quickshell 0.3.1 —
the build both DMS and Noctalia run on — is offered that global and never
binds it, binding `zwlr_foreign_toplevel_manager_v1` instead. Its binary does
carry both sets of symbols (checked 2026-09-16 on the dev VM:
`ext_foreign_toplevel_list_v1`, `ext_foreign_toplevel_handle_v1`,
`ext_foreign_toplevel_image_capture_source_manager_v1` and
`zwlr_foreign_toplevel_manager_v1` are all present), so it may bind the `ext-`
list specifically for the capture path even though its `ToplevelManager` does
not.

**Measure that before building anything**: if quickshell never binds the `ext-`
list at all, this whole path is unreachable for the motivating client, and the
useful work is instead whatever it *does* speak — which for per-window
thumbnails would have to be a region capture out of the output, since
`wlr-screencopy-unstable-v1` has no toplevel source either.

Rough size: M.
