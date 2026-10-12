---
title: "Headless VNC shows no mouse: pointer cursor sessions refused"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Headless VNC shows no mouse: pointer cursor sessions refused

Filed 2026-10-12 from the try-scoot review (scoot-sh/try-scoot PR #6). Serves **daily-drive** and the **webtop/try-scoot** path: a desktop opened in a browser shows no pointer, which is the first thing a stranger notices.

## The gap

Run the try-scoot image (`docker run --rm -p 127.0.0.1:6080:6080 ghcr.io/scoot-sh/try-scoot`), open noVNC, move the mouse: no pointer. Measured by the try-scoot author on scoot `722afd2`: zero VNC cursor pseudo-encoding rects over the RFB stream, and two full frames with the pointer at different positions are byte-identical. `scoot msg screenshot` draws the arrow (its `cursor` field defaults to included), so every compositor-side check looks fine.

Mechanism, read in source (`crates/scoot/src/compositor/screencopy.rs:195-199`, `render/capture_cursor.rs:8-9`) and confirmed by the independent reviewer:

1. wayvnc captures through `ext-image-copy-capture-v1` (scoot publishes only that, not wlr-screencopy) and, run without `-r`, asks for no painted cursor, so scoot correctly serves cursorless frames.
2. The cursor *image* travels in a separate pointer cursor session. scoot refuses `create_pointer_cursor_session` outright (Smithay's default `cursor_capture_constraints` returns `None`), on the stated ground that "no client measured here asks for one". wayvnc does: it is how it learns the cursor shape to send as a VNC cursor.
3. With no shape and no painted pixels, noVNC applies its default `cursor: none` and hides its own local pointer.

try-scoot ships a workaround, noVNC `show_dot=true` (a dot). The try-scoot author reports `wayvnc -r` (paint cursors) left frames byte-identical; the review could not confirm that, and the fix round re-probes it.

## What to do

- Implement `cursor_capture_constraints` so pointer cursor sessions are served: render the compositor's current cursor image (the theme cursor scoot already loads) into the client's buffer, update on cursor shape and hotspot change, and drop the session cleanly on output removal. Measure first: it is a second render target with its own lifecycle, so keep it off the hot path when no cursor session exists (no allocation per frame), and say what it costs when one does.
- Check that `paint_cursors` (the painted-in alternative) really lands pointer pixels in headless and nested captures: `capture_cursor.rs` implements per-request patching, so a client that asks for it should get it on every tier. If wayvnc `-r` fails to get it, find out where it is lost.
- Cover headless, nested and tty. Test with wayvnc against a headless session by diffing raw frames at two pointer positions, as the try-scoot author did.
- Update `screencopy.rs` module docs: the "no client measured asks for one" line is now false.

## Not in this ticket

A VNC server inside scoot; changing try-scoot (it keeps the dot until this lands).
