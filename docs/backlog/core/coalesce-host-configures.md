---
title: "Every host configure is acted on immediately, so a drag mints a mode per pixel step"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Every host configure is acted on immediately, so a drag mints a mode per pixel step

Found by review of PR #151 (the `--nested` resize fix), confirmed live:
`wlr-randr` on a nested session reports **2 modes after the first configure
and 4 after three resizes** — including one size that never rendered.

A host resize is not one event. Dragging a webtop browser window from 800px
to 1900px sends a configure per pixel step, and scoot acts on every one.
Each *distinct* size:

- appends to `Output::modes`, which nothing prunes;
- mints a `zwlr_output_mode_v1` per output-management client
  (`output_management.rs:452-461`);
- makes the next refresh slower, because `update()`'s scan is linear — so
  the cost over a drag is **O(N²)**.

That drag is roughly **1100 modes and 1100 protocol objects per client**,
on the deployment target `README.md` names, in the single most ordinary
interaction it has. Nothing bounds it and nothing frees it for the life of
the session.

## Why this is worth doing properly rather than patching

Coalescing configures to the next render tick fixes four things that were
filed separately, which is the tell that it is the right shape:

1. **This.** One resize per frame instead of one per pixel step, so the mode
   list stops tracking mouse movement.
2. **The GLES resize stutter.** `--nested --renderer gles` costs **16.6 ms**
   per resize against pixman's 37 µs — a whole 60 Hz frame. Per *distinct
   size* that is tolerable; per *pixel step of a drag* it is not, and
   coalescing is what makes the difference. `docs/tty.md` names the deeper
   fix (resize the GLES target in place); this is the cheaper half.
3. **The unbounded product.** 8192×32767 passes both axis guards today —
   ~2 GB of pool plus ~1 GB of pixman target — and since #151 that is
   reachable on *any* configure rather than only the first.
4. **The benchmark's blind spot.** `headless/bench.rs` measures the renderer
   half only, excluding the pool rebuild (memfd, ftruncate, mmap, two
   buffers, ~2000 page faults on first write at 1280×800). The "37 µs" floor
   is not the nested total, and a per-frame budget is the honest unit to
   measure against.

The alternative — calling `delete_mode` on the nested path — fixes only (1),
and leaves the compositor doing a full pool rebuild per pixel of a drag.

## Watch for

`wl_output` has no un-prefer. `set_mode(new)` marks the new mode preferred
before a failed resize restores the old one, so a client bound across a
failed resize can see **two** modes flagged preferred, and the failed mode
stays in `Output::modes` for later binds. SUSPECTED, from the same review;
worth confirming while in here, because coalescing changes how often that
window opens.
