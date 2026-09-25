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
