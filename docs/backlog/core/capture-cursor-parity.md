---
title: "Captures lose the pointer on the GPU scanout tier (cursor plane) — make capture cursor behaviour tier-independent"
status: "open"
area: "core"
priority: "high"
blocked: null
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
