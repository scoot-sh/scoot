---
title: "Headless VNC shows no mouse: pointer cursor sessions refused"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
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

## Resolution (2026-10-27)

Pointer cursor sessions are served on every backend (`screencopy/cursor_session.rs`,
the image from `render/capture_cursor/image.rs`). Measured before the change on
the dev VM with wayvnc 0.10.1 and a raw RFB client (`Raw` + `Cursor` encodings):
no cursor rect and no screen update across two pointer moves. After: one cursor
rect on connect (16x16, hotspot 1,1), none on motion alone, a new one (32x32)
after a reload changed `cursor_size`.

- **`wayvnc -r` does land pointer pixels**: frames differ between pointer
  positions and small damage rects land at the old and new spot (headless,
  `-r`, same RFB client). The try-scoot observation does not reproduce on
  this build. With the cursor session served, `-r` is now harmful: wayvnc
  paints the pointer and also sends it as a cursor, so a viewer shows two.
  The remote-desktop page says to leave it off.
- **Cost**: nothing while no client holds a session beyond an empty-list
  check per motion event. With one, a motion event arms the frame tick and the
  tick sends a deduplicated position event; the image is rendered (one
  cursor-sized offscreen render, pooled target, shared across sessions) only
  when the cursor's image changed.
- The cursor's image serial (`State::cursor_image_serial`) is separate from
  `cursor_serial`, which keeps meaning "moved or changed".
