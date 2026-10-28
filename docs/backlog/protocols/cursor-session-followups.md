---
title: "Pointer cursor session follow-ups from the #774 review"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Pointer cursor session follow-ups from the #774 review

Filed 2026-10-27. Serves **computer use** (a VNC viewer is how an agent or a person watches the VM; the pointer has to be right there).

PR #774 served `create_pointer_cursor_session` on every backend. The independent review passed it and left these gaps, none blocking.

## The gap

- ~~**No scale-change test.**~~ Done: `a_scale_change_is_announced_and_the_old_buffer_refused_then_the_new_size_is_served` drives a live session through 1.0 to 1.5 and back.
- **GPU scanout tier unexercised.** The GLES path ran only through soft-egl in the harness; `--renderer gpu` fell back to cpu on the dev VM. Needs a real-GPU run.
- ~~**Client cursor hotspot at a fractional scale** can be 1 px off.~~ Done: a client cursor surface is placed at `round(pointer * scale) - round(hotspot * scale)` (`cursor::element_location`), so the image's hotspot is constant and matches the position the session reports; pinned at 1, 1.25 and 1.5.
- **One pooled target is shared** by the cursor image and a `paint_cursors` region patch, so a shape change while both are live on one output rebuilds the target. Rare.
- **A reload can serve one redundant frame** with the old image before the async cursor rebuild lands. Harmless.
- **No per-client cap on cursor sessions**, the same as for capture sessions today.

## What to do

- Run the cursor session on a real GPU scanout output and record the result.
- Decide the session cap together with the capture-session cap, not separately.

## Not in this ticket

Changing the per-output render-once sharing, and the wayvnc `-r` guidance (the pointer is drawn twice with it; the docs already say to leave it off).
