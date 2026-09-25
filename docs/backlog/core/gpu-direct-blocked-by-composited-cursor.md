---
title: "GPU scanout: on a CRTC with no cursor plane, a visible pointer blocks primary-direct for every fullscreen window"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# A composited cursor blocks primary-direct

Found 2026-09-25 on the Apple M2 under Asahi Linux (`Asahi.md`, Test 5
results). Serves **daily-drive**: fullscreen video and games on laptops
whose display controller has no cursor plane.

## What was seen

`apple,dcp` exposes one primary plane, one overlay plane and **no cursor
plane** (`drm: scanout cursor planes cursor_planes=0 overlay_planes=1`).
The pointer is a `Kind::Cursor` `MemoryRenderBufferRenderElement`
(`cursor.rs`). Smithay (pinned fork `43f50eb`) does allow `Kind::Cursor` on
an overlay, but `element_config` needs an underlying storage it can export
as a framebuffer, and a memory buffer has none. So the attempt fails
silently and the cursor goes to the render list. Smithay tries the primary
plane only for the last element, and only when nothing above it was
rendered (`remaining_elements == 1 && primary_plane_elements.is_empty()`,
`drm/compositor/mod.rs` ~1995). So while the pointer is drawn over a
fullscreen window, that window never gets a primary attempt at all. With
`scanout: primary-direct eligibility changed … to=Eligible` and `scanout
steering changed steer=Sent` logged, a fullscreen `es2gears_wayland` and
`vkcube` composited every frame (two elements, zero `testing direct
scan-out` lines).

mpv went direct only after it hid its pointer (`set_cursor(serial, nil)`,
`--cursor-autohide=always`, once it had pointer focus). Direct scanout then
cut compositor CPU about 60% (10–11 against 27–28 jiffies per 10 s for the
same fullscreen video). A single-output laptop has nowhere to park the
pointer, so a client that never hides it never goes direct.

`docs/tty.md` already documents "a cursor with no plane of its own makes
that frame composite". The ticket is that on this hardware, that means
every frame.

## Options (pick one after measuring)

1. **Hide the pointer after inactivity** while a fullscreen window covers
   the output and has pointer focus (`[appearance] cursor_hide_after_ms` or
   similar, default off or on to taste). This is the cheapest option, is
   hardware-independent, and matches what players do themselves. The next
   motion shows the pointer again, and that frame composites.
2. **Put the cursor on the overlay plane.** This needs the cursor image in
   a GBM `LINEAR` `AR24` buffer: DCP's overlay takes `AR24` but no `XR24`,
   has zpos 1 and so sits above the primary, and accepts `LINEAR` only.
   Smithay does that copy only for cursor-type planes, so it means either a
   scoot-side `Kind::Cursor` element backed by a dma-buf or a Smithay
   change. It competes with
   [windows on overlay planes](gpu-overlay-window-candidates.md) for the
   one overlay. The captures table in `docs/tty.md` already has a
   "cursor on an overlay plane" row.

Either one must keep captures correct (`capture_cursor.rs`). Verify on the
Asahi machine with debugfs `dri/2/state` (plane 35 on the client's fb) and
mpv's `presented` flags (`9` = `vsync | zero_copy`).
