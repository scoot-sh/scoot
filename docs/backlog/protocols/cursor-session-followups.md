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

- **No scale-change test.** `ImageKey.scale` and the `refresh_capture_constraints` hook have no test. A scale change while a session is live is untested.
- **GPU scanout tier unexercised.** The GLES path ran only through soft-egl in the harness; `--renderer gpu` fell back to cpu on the dev VM. Needs a real-GPU run.
- **Client cursor hotspot at a fractional scale** can be 1 px off (cosmetic).
- **One pooled target is shared** by the cursor image and a `paint_cursors` region patch, so a shape change while both are live on one output rebuilds the target. Rare.
- **A reload can serve one redundant frame** with the old image before the async cursor rebuild lands. Harmless.
- **No per-client cap on cursor sessions**, the same as for capture sessions today.

## What to do

- Add a wire test for a scale change with a live session: new constraints, `buffer_constraints` on the waiting frame, then a good frame at the new size.
- Run the cursor session on a real GPU scanout output and record the result.
- Round the hotspot the way the cursor draw does, and pin it at scale 1.25 and 1.5.
- Decide the session cap together with the capture-session cap, not separately.

## Not in this ticket

Changing the per-output render-once sharing, and the wayvnc `-r` guidance (the pointer is drawn twice with it; the docs already say to leave it off).
