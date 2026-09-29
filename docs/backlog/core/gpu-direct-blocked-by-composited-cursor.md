---
title: "GPU scanout: on a CRTC with no cursor plane, a visible pointer denies every fullscreen window a primary-direct attempt"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# A composited cursor blocks primary-direct

Found 2026-09-25 on the Apple M2 under Asahi Linux (`Asahi.md`, Test 5
results). Serves **daily-drive**: fullscreen video and games on machines
whose display controller has no cursor plane.

## What was seen

`apple,dcp` exposes one primary plane, one overlay plane and **no cursor
plane** (`drm: scanout cursor planes cursor_planes=0 overlay_planes=1`).
The pointer is a `Kind::Cursor` `MemoryRenderBufferRenderElement`
(`cursor.rs`). Smithay allows `Kind::Cursor` on an overlay, but at the
pinned fork `43f50eb` a memory buffer cannot become a framebuffer:
`UnderlyingStorage` has only `Wayland` and `Memory` variants, and
`ExportBuffer::from_underlying_storage` maps `Memory` to `None`
(`drm/exporter/mod.rs` ~38). So the overlay attempt fails silently and
the cursor goes to the render list. The trace log agrees:
- 6733 cursor-plane checks ("no cursor state");
- 4773 overlay skips for elements that are not scanout candidates or
  cursors;
- about 1960 cursor elements reaching the overlay step and failing with
  no log line;
- `plane[40] fb=0` in debugfs throughout.

Smithay tries the primary plane only for the last element, and only when
nothing above it was rendered (`remaining_elements == 1 &&
primary_plane_elements.is_empty()`, `drm/compositor/mod.rs` ~1995). So
**while the pointer is drawn over a fullscreen window, that window gets
no primary attempt at all**, even with `scanout: primary-direct
eligibility changed … to=Eligible` and `steer=Sent` logged. A single-output
laptop has nowhere to park the pointer, so this holds for as long as the
pointer is visible.

With the pointer hidden, mpv (`--cursor-autohide=always`, which sends
`set_cursor(serial, nil)`) went direct on its `LINEAR` 2561x1601 buffer.
That cut compositor CPU about 60%: 10–11 against 27–28 jiffies per 10 s
for the same fullscreen video.

**Hiding the pointer is necessary here, not sufficient.** The buffer
still has to qualify: a format and modifier the primary takes (`LINEAR`
only on DCP), and a size matching the mode. Of the three clients tried,
only mpv's buffer qualified. `vkcube` (tiled-compressed) and
`es2gears_wayland` (a 1707x1067 buffer at `set_buffer_scale(1)`, which
would need a 1.5x plane upscale, untested) would have composited even with
no pointer. So they are not evidence for this ticket.

`docs/tty.md` already documents "a cursor with no plane of its own makes
that frame composite". The ticket is that on this hardware, that means
every frame with the pointer visible.

## Options (pick one after measuring)

1. **Hide the pointer after inactivity** while a fullscreen window covers
   the output and has pointer focus (`[appearance] cursor_hide_after_ms` or
   similar, default off or on to taste). This is the cheapest option, is
   hardware-independent, and matches what players do themselves. The next
   motion shows the pointer again, and that frame composites.
2. **Put the cursor on the overlay plane.** Format is not the blocker:
   DCP's overlay takes `AR24` (and `AR30`, `AB24` and YUV), `LINEAR` only,
   at a fixed zpos of 1 above the primary. The blocker is Smithay. It
   cannot export a memory buffer as a framebuffer (above), and its GBM
   exporter also rejects `wl_shm` buffers (`drm/exporter/gbm.rs`
   ~105–118). A scoot-side `Kind::Cursor` element backed by a dma-buf
   therefore needs **a change in scoot's Smithay fork**, following the
   `docs/forks.md` process, with no upstream PRs. It also competes with
   [windows on overlay planes](gpu-overlay-window-candidates.md) for the one
   overlay. The captures table in `docs/tty.md` already has a "cursor on an
   overlay plane" row.

Either one must keep captures correct (`capture_cursor.rs`). Verify on the
Asahi machine with debugfs `dri/2/state` (plane 35 on the client's fb) and
mpv's `presented` flags (`9` = `vsync | zero_copy`), using a client whose
buffer qualifies.

## Progress — option 1 shipped, ticket stays open

Option 1 (hide the pointer after inactivity) is implemented, behind the
opt-in `[appearance] cursor_hide_after_ms` (default `0` = never hides):
while a fullscreen window covers the output the pointer is on, and the
pointer is over that window itself, the compositor hides its pointer after
the configured delay; the next motion, button or scroll shows it again and
that frame composites. Hiding suppresses the cursor-element gathering
(`State::cursor_location`), never the client's own cursor status, so a
hidden cursor disturbs no plane assignment, a reshown one composites
until the pointer hides again (headless: one frame; on Asahi the whole delay, Test 14), and captures (IPC screenshots,
`ext-image-copy-capture-v1`) read a hidden pointer as hidden. Never hides
behind the session lock; keys do not reset the wait. Headless-verified
(`compositor::cursor_hide::tests`: synthetic-clock hide, race, reshow
pixels, disarm on un-fullscreen/close/workspace-switch/lock/VT-pause
wiring, uncovered-output, zero-means-off); motion hot path +~14 ns/event
with the feature on, nothing measurable with it off.

**Asahi plane assignment verified 2026-09-29 (`Asahi.md`, Test 14, `main` at
`b3f087b43`).** With `cursor_hide_after_ms = 1000` and mpv left showing its
own pointer, fullscreen mpv reached plane 35 on its own `XR30 2561x1601`
fb after the pointer sat still (1036 `testing direct scan-out`, all on
plane 35, all assigned); one pointer motion put scoot's `AR24` swapchain
back for the whole `cursor_hide_after_ms` delay (direct attempts stopped for
1.035 s in the trace, about 31 frames) before the client fb returned. Unset: no primary
attempt at all, plane 35 stayed on `AR24`. Compositor CPU: 13-14 against
29-33 jiffies per 10 s (1080p30 clip, two rounds each), 21 against 37-40 for
a 60 fps 2560x1600 clip.

**`zero_copy` under the hide verified 2026-09-29 (`Asahi.md`, Test 15,
same binary as Test 14).** Four alternating `WAYLAND_DEBUG=1` sessions ran
fullscreen mpv (`--cursor-autohide=no`, which sets a visible arrow once
through `wp_cursor_shape_v1` and sends no `set_cursor`). With
`cursor_hide_after_ms = 1000`, mpv's `presented` flags were `9`
(`vsync | zero_copy`) 733 and 732 times. Each count equals scoot's
`successfully assigned … to plane::Handle(35)` lines in the same session.
The first `9` came 1.010-1.018 s after the last motion in phase B, and about 1.0 s in phase D. Continuous motion
(every 0.25 s) gave only `1`, starting within a frame or two of the first
move. Unset, the sessions saw 1156 and 1158 `1`s and not one `9`. No
`discarded` and no other flag value was seen. **This closes option 1's
Asahi verification.** It covers one client (mpv), a 1000 ms delay,
motion-driven reshows only, and scale 1.5 on eDP-1.

Remaining: option 2 (cursor on the overlay plane, needs a Smithay-fork
change per `docs/forks.md`). This Asahi kernel (7.1.13) now exposes **two**
overlays per CRTC, so option 2 need not compete with window overlays
there. The option-2 text above, which says it competes for the one overlay,
predates this. The ticket keeps `status: "open"` until option 2 lands or
is ruled out.
