---
title: "A new buffer scale is ignored until a new buffer is attached — RESOLVED (in scoot, via a Smithay fork)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A new buffer scale is ignored until a new buffer is attached

Found 2026-09-27 building scootbg's images
([images-decode-and-fit-done.md](../../scootbg/backlog/resolved/images-decode-and-fit-done.md#found-along-the-way)).

`wl_surface.set_buffer_scale` (and `set_buffer_transform`) are
double-buffered state that "applies on the next `wl_surface.commit`", with
or without a new `attach`. The pinned Smithay fork (`74edbf32`) reads them
only when a new buffer is attached: `RendererSurfaceState::update_buffer`
(`src/backend/renderer/utils/wayland.rs`, the `BufferAssignment::NewBuffer`
arm, around lines 154–167) sets `self.buffer_scale` and
`self.buffer_transform` there and nowhere else, so a commit that changes
only the scale keeps rendering the buffer at the old one.

**Reproduced on scoot** (`scoot --headless`, pixman): a client shows a
1600×1000 buffer at scale 1, scoot's `scale = 2.0` is reloaded (the
surface is reconfigured to 800×500, the same buffer is now right at scale
2), and the client commits `set_buffer_scale(2)` with no new attach
(trace: `set_buffer_scale(2)`, `set_opaque_region`, `damage_buffer`,
`commit`). The screenshot shows the buffer's top-left quarter blown up to
the whole output, until the client attaches a buffer again.

**Who meets it:** a client that keeps its buffer when only the scale
changes (a scale and a mode that change together, as above). Most clients
redraw at a new scale and attach, so it rarely shows. scootbg now attaches
its buffer again whenever it sends a new scale without a new buffer
(`crates/scootbg/src/daemon/canvas.rs`), which is correct everywhere, and
`crates/scootbg/tests/image.rs::a_new_scale_redraws_at_the_real_pixel_size`
fails without that workaround. Since scootbg draws with
`wp_fractional_scale_v1` (2026-09-27), it keeps buffer scale 1 on scoot
and meets this only on a compositor without that protocol (the test runs
with it left unbound). A new `wp_viewport` destination alone is applied
without a new buffer (`update_buffer` works the surface view out at every
commit; `crates/scootbg/tests/scale.rs` checks it by screenshot).

**Fix:** in the fork, apply the cached `buffer_scale` and
`buffer_transform` on every commit that has a buffer, not only on a new
one (and recompute the surface view, which already happens each commit),
as a scoot-sh/smithay commit listed in `dev/forks.md`; then a
compositor-side test attaching once and changing only the scale. Not sent
upstream from here (see `CLAUDE.md`).
