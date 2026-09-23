---
title: "Captures lose the pointer on the GPU scanout tier (cursor plane) — make capture cursor behaviour tier-independent — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Captures lose the pointer on the GPU scanout tier — RESOLVED

RESOLVED 2026-09-23 (PR #231, branch `capture-cursor-parity`; review
round 1 fixes included). The pointer in a
capture is now the request's, on every backend and renderer:
`ext-image-copy-capture-v1` honours `paint_cursors` (it was ignored
everywhere), and IPC `screenshot` has an optional `cursor` field, default
drawn in (`scoot_ipc::SCREENSHOT_CURSOR_DEFAULT`, `scootctl screenshot
--no-cursor`; the user confirmed the default), added without a
`PROTOCOL_VERSION` bump.

**A premise corrected on the way.** The dispatch took "on every other tier
the cursor is composited, so captures show it" as given. Only `--tty`'s
dumb tier composites it: `--headless` and `--nested` never draw a cursor at
all (`gather_elements` was gated on `tty.is_some()`). So making captures
tier-independent changed two behaviours beyond the scanout tier: a
`--headless`/`--nested` IPC screenshot now shows the pointer (at the output
centre on a fresh session), and plain `grim` on the dumb tier no longer
does (`grim -c` does). The default is one constant if it should differ.

- **How: the cursor's region is re-rendered, not blended onto the copy**
  (`render/capture_cursor.rs`). Where the frame a capture reads does not
  already hold the cursor as asked, the region -- where the cursor is now
  (when asked for), where the frame composited it, and where a cursor on an
  overlay plane may have left an underlay hole -- is drawn again from the
  frame's own element list, with or without the cursor, by the session's
  own renderer into a cursor-sized offscreen target, and written over the
  captured copy. That is the only shape that can take a composited cursor
  *out*, and it cannot produce two cursors (a forced composite, a plane
  that refused).
- **Underlay holes, fixed in review.** The first cut filled a hole only
  when the pointer was asked for at the same place: `patch_region` returned
  the composited footprint alone for `!want`, and a cursor on a plane
  recorded none, so plain `grim` / `--no-cursor` (and the old position of
  a pointer that moved) would have shown the hole. At the pinned Smithay an
  element on an overlay whose zpos is below the primary's gets a hole-punch
  element in the primary (`render_frame`, `drm/compositor/mod.rs` ~2224),
  and `Kind::Cursor` is an overlay candidate (~3733). The record now keeps
  the footprint of every overlay-planed cursor element (`on_overlay`) and
  the region always includes it. The cursor plane is not recorded: only the
  overlay arm punches holes. Pinned with a synthetic record (a punched hole,
  filled both ways, and at a position the pointer has since left); not seen
  on hardware (virtio has no overlay plane). `Rounded::relocate` moves the corner clip with
  the element, without which a relocated rounded window lost its cut (a
  mutation test caught 14 px).
- **Source of truth per frame** (`CursorInFrame`): the drawn list on the
  persistent-framebuffer pipelines, and the `DrmCompositor`'s own
  `cursor_element`/`overlay_elements` beside the recorded swapchain slot on
  the scanout tier (written only with the slot).
- **A cursor-painting session is due when only the cursor changes**
  (`State::cursor_serial`; `cursor_changed` wakes the tick where frames draw
  no cursor). Every cursor path goes through `cursor_changed` -- motion,
  `set_cursor`/cursor shape, the tablet tool cursor, a commit to the cursor
  surface tree, its destruction, the lock's reset, a cursor reload (the
  last four fixed in review). Conversely a redraw the cursor alone asked for
  (`request_cursor_render`) no longer moves `frame_serial`, so a session
  that did not ask for the pointer is not re-served an identical frame when
  only the pointer moves (on the dumb tier: 209 frames in 5 s before, 1
  after). Multi-output: the cursor is placed at its position on each
  output, and is in no other output's frame or capture.
- **No per-frame allocation of its own for a stream** (review): the region's
  target and damage tracker, the cursor/gather/relocated element lists
  (`PatchPool`, per pipeline) and the pixel buffer (`Backend::recycle_patch`)
  are reused; the target is rebuilt only when the region size or scale
  changes. pixman reads its target's bits directly. Still per capture:
  Smithay's per-bind FBO and per-read PBO under GLES at the pinned rev, and
  the gather sources' own lists the frame path shares.

## Evidence (commands, SHAs and raw paths are in the PR)

- Harness: oracle tests (`screencopy/tests/cursor.rs`,
  `cursor/tests.rs`, `outputs/per_output.rs`) -- every capture equals a
  whole-frame composite with or without the cursor, byte for byte, under
  pixman and GLES: headless add, dumb-tier removal, moved-since-frame (one
  cursor), rounded corner, fractional scale, output edge, hidden, named
  shape, client cursor surface with hotspot/offset/subsurface, locked,
  Xrgb alpha, pointer-only re-serve, no render loop, multi-output; and
  after review: an underlay hole filled both ways and after a move, a
  cursor-only redraw re-serving only pointer-requesting sessions, a stream
  reusing one target and one pixel buffer, every cursor path moving the
  cursor serial. Mutations (drop the Rounded relocation, the union, the
  patch, the output-local placement, the cursor serial, the tick, the hole
  in either branch, the cursor-only render) each fail them.
- Live, dev VM virtio-gpu (`cursor_planes=1`, plane 34 tracking the
  pointer): scanout vs dumb tier at four pointer positions -- IPC equals
  `grim` both ways on both tiers; with vs without the cursor differs in
  exactly the 136 px of the arrow in its 16x16 box; the arrow's pixels are
  identical across tiers, and the tiers differ only in the background clear
  colour (20,20,26 vs 20,20,25 -- 826184 px, pre-existing). Before: every
  scanout-tier capture cursorless, every dumb-tier capture cursored.
- Primary-direct (PR #228 path): a fullscreen dumb-buffer client on the
  primary (fb MATCH), 6 IPC captures each forcing a composite, each holding
  exactly the 136 arrow pixels at the pointer; the same scene is
  byte-identical to the dumb tier.
- Cost (llvmpipe): the region is 0.4-0.9 ms per pointer-requesting capture
  on the scanout tier with the cursor on its plane; compositor CPU per 40
  IPC screenshots 26 → 42-46 jiffies at first, 29 → 37-39 once pooled.
  Wall latency is not a stable measure of it: the first cut's fresh-process
  +3.5 ms median was +1.5 ms after pooling, and interleaving cursor and
  no-cursor captures in one process makes the cursor captures the faster
  ones (review: 9.1 against 10.2 ms; here: p50 10.9-11.3 against 12.4-13.0)
  -- it tracks page faults from whole-frame per-capture allocations both
  modes pay, filed as
  [capture-whole-frame-allocations](../core/capture-whole-frame-allocations.md).
  A `paint_cursors` stream there delivers ~4% fewer frames (436-439 against
  453-457 in 10 s), unchanged by the pooling. Unchanged with `--no-cursor`,
  on the dumb tier and headless. Hot path: `cursor_changed` 1 ns and
  `CursorInFrame::of` 9 ns (release), motion A/B indistinguishable. No real
  GPU measured.
- Not live: `--nested` (same frame shape as headless, harness-covered),
  an underlay or partial plane assignment (virtio has no overlay plane;
  unit- and harness-covered with a synthetic record).

Original entry below, kept verbatim.

---


# Captures lose the pointer on the GPU scanout tier

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **agent-driven
computer use** first (an agent reading a screenshot needs to see where the
pointer is, and needs the answer not to change with the renderer), and
daily-drive second (screen recordings / screen sharing).

## What is wrong

Since PR #216 the cursor rides its own KMS plane on `--tty --renderer gles`
in a `gpu-scanout` build wherever the CRTC has one, and captures (IPC
`screenshot`, `ext-image-copy-capture-v1`) read the primary plane only, so
they show the screen **without** the pointer. On every other tier the
cursor is composited and captures show it. The README documents this as
"absent from captures by design". The same agent script therefore sees
different pixels depending on which renderer the session runs — exactly the
targeting-fidelity inconsistency computer use cannot absorb.

**A worse variant, not yet seen on hardware (noted 2026-09-22, PR #222).**
Since the scanout exporter admits client dma-bufs, a client cursor surface
backed by a dma-buf can ride an overlay plane where the cursor plane cannot
take it. Where that overlay sits *below* the primary (an underlay, zpos
lower than the primary's), Smithay only puts an *opaque* element there and
punches a transparent hole in the primary above it -- so the capture, which
reads the swapchain slot, shows a transparent cut-out where the pointer is
rather than simply lacking it. Needs an opaque dma-buf cursor and
underlay-capable hardware (neither on the dev VM, which has no overlay).
Whatever this ticket does for the cursor must cover the underlay case too.

## What to do

Make capture cursor behaviour a property of the *request*, not the tier:

- `ext-image-copy-capture-v1` already has the knob: the capture source's
  `paint_cursors` option. Honour it on every tier (composite the cursor into
  the capture when asked, omit it when not) — check what scoot does with the
  option today on the pixman tier too; it may already be ignored there.
- IPC `screenshot`: pick one documented default (the pixman tier's
  behaviour today is "cursor included") and make the scanout tier match it,
  optionally with an explicit request field if the protocol wants one —
  `scoot-ipc` changes need `PROTOCOL_VERSION` thought and `docs/ipc.md`.
- Implementation shape is the implementer's call, but it must not cost a
  full recomposite per capture on the hot path: drawing the cursor element
  onto the captured copy (the capture already owns a CPU buffer) is the
  cheap form.

## Evidence

Byte-level: a capture on the scanout tier with a plane-assigned cursor, with
and without the cursor requested, diffed against the pixman tier. The dev
VM's virtio-gpu *does* plane-assign the cursor (`cursor_planes=1`), so this
is fully verifiable there.
