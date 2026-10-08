---
title: "dma-buf capture buffers for ext-image-copy-capture-v1 — RESOLVED (documented-deferred, don't build: no measured client need)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# dma-buf capture buffers for `ext-image-copy-capture-v1` — RESOLVED (don't build)

RESOLVED 2026-09-28 (verdict-only PR, no code change). **Don't build:
no measured client need.** The ticket set its own bar — "a measured
client need, which is the same bar `screencopy.rs`'s module doc has
always set" — and re-verified against the current tree on 2026-09-28,
nothing meets it: `grim` captures into `wl_shm`, quickshell's
`ScreencopyView` captures into `wl_shm` (its dmabuf dependency is
feedback readiness, not capture buffers), and the tree holds no other
capture consumer. All four technical blockers re-verify as stated,
with two record corrections below. `docs/protocols.md` already states
shm-only honestly; no user-facing behavior changed.

## Methodology

Code reading against current `main` (`392c3f25`) plus the pinned
Smithay fork source — not relayed from the ticket text. Base commit
for every claim: `392c3f25` (merge of PR #311). No hardware run: this
is a docs-only verdict, and every premise is a static property of the
tree or an already-recorded measurement cited by path.

## What was checked

- **The bar still stands.** `screencopy.rs`'s module doc (:186-212)
  still sets it: "`wl_shm` only for *capture*" with the deliberate
  reasoning, and points at this item as the place with "its own write
  path and its own before/after numbers".
- **(a) No dma-buf write path in `deliver`.** Confirmed, with the code
  moved one level since filing: `deliver` (:1103) reads the
  framebuffer back once and fans out through `write_due_captures`
  (:1200) into `write_capture` (:1335), whose doc line is now
  "Writes a `width` x `height` BGRA image into a client's shm buffer"
  and whose only write primitive is `with_buffer_contents_mut`
  (:1357). `constraints` (:1080) still returns `dma: None` (:1088).
- **(b) Render-vs-texture format sets still differ.** Confirmed:
  `tty/scanout.rs:412-422` still keeps the render set
  (`dmabuf_render_formats`) for `DrmCompositor::new` and says it
  never reaches the client advertisement, which is derived from
  `dmabuf_texture_formats` in `dmabuf.rs`. New nuance since filing:
  `Backend::dmabuf_render_formats()` (`render.rs:855`) answers `Some`
  only for the offscreen GLES pipeline — pixman *and* the scanout
  tier answer `None` — so a capture offer would need a format source
  that does not exist on two of the three pipelines, not just a
  different one.
- **(c) `DmabufConstraints` still needs a `DrmNode`.** Confirmed
  against the pinned fork source (see the rev correction below):
  `image_copy_capture/mod.rs:140-145` declares `pub node: DrmNode`,
  non-optional. `Backend::render_node()` (`render.rs:911-918`) is
  `None` for pixman ("no device at all"), so the GPU-less target the
  ticket names still has nothing to put in that field.
- **(d) The pixman bind-target cache-drain landmine.** Confirmed, and
  the note grew since filing: `dmabuf.rs:1125-1152` ("Maintenance
  hazard") now distinguishes the GPU scanout tier's
  `Backend::capture`, which binds the swapchain slot's `Dmabuf`
  through `GlesRenderer` — safe, because GLES `cleanup` retains on
  `!is_gone()` alone (fork `gles/mod.rs:819-823`, re-checked) — from
  the still-real pixman hazard (fork `pixman/mod.rs:807-815`,
  re-checked: the second `retain` keeps only entries with a live
  dmabuf, evicting the bound-target cache wholesale). A pixman
  screencopy path would still have to narrow that drain in the same
  change.
- **`grim` still captures into `wl_shm`.** Derivation, not
  recollection: scoot's capture constraints offer shm only
  (`Xrgb8888`/`Argb8888`), and `grim` 1.5.0 captures succeed against
  scoot everywhere it is measured (`dev/benches/benchmarks.md` shot-grim
  rows, `CHANGELOG.md`, the `gles-capture-leaks-a-frame-per-shot`
  record). A client needing dmabuf buffers could not complete a
  single one of those captures.
- **quickshell's `ScreencopyView` still needs only the global.**
  Wire-log proof, not recollection:
  `screencopy-shell-thumbnails-fallback-done.md:92-130` — with real
  dmabuf feedback the overview view binds the ext globals and
  captures into `wl_shm_pool.create_buffer` (`Xrgb8888`), with zero
  `create_params`/`create_immed` lines; and :53-69 traces readiness
  to `feedbackDone()` → `mDmabufFormatsReady = true` on the feedback
  `done` alone, with quickshell falling back to shm buffer creation
  past it. No capture buffer is ever allocated from a dmabuf.
- **No new capture consumer since 2026-09-19.** Tree-wide survey for
  screen-sharing, encoder and recorder demand (`wf-recorder`,
  `pipewire`, `screen-share`, `obs`, `encoder`, portal ScreenCast
  paths): the only hits are hypothetical — "whatever
  `wf-recorder`/OBS do with an `Xrgb8888` screencopy buffer"
  (`screencopy-xrgb-alpha-forcing-done.md:60-62`, an open question
  for a different ticket's decision, not a measurement) — plus the
  already-shipped capture work. The plausible pipeline,
  ScreenCast through xdg-desktop-portal-wlr ≥ 0.8.0, is configured
  (`resources/scoot-portals.conf`) but unexercised: live portal
  proof is impossible on the dev VM (no portal stack installed).
  Unmeasured is unmeasured — that is the reopen trigger, not a need.
- **`docs/protocols.md` already states shm-only honestly**
  (:1393-1398: "`wl_shm` buffers only, `Xrgb8888` or `Argb8888`" with
  the import-vs-render-into distinction). No doc change needed there.

## Record corrections (stale premises fixed, not preserved)

- **Fork rev.** The brief for this re-verification said "pinned fork
  rev `6ab8b4a2`". The current pin is `e7130254`
  (`crates/scoot/Cargo.toml`, `scoot/xwayland-selection-dnd`):
  `6ab8b4a2` was pinned briefly on the side branch
  `scoot/buffer-scale-without-new-buffer` and nothing pins it now —
  `e7130254` is content-identical for the fix it carried
  (`dev/forks.md`, `buffer-scale-without-new-buffer-done.md`). Every
  fork claim above was verified at the `e7130254` checkout
  (`~/.cargo/git/checkouts/smithay-*/e713025`), not at `6ab8b4a2`.
- **`(b)` widened** (the `dmabuf_render_formats() → None` second and
  third pipelines above) and **`(d)` grown** (the scanout-tier
  safe case now documented beside the hazard) — both are current-doc
  facts the ticket's 2026-09-19 wording predates.

## Verdict

Documented-deferred, don't build — the precedents are PRs
#282/#285/#308 (verdict-only resolutions with the evidence
re-derived, not relayed). The cost side stands (a new write path per
renderer, a conditional offer that is `None` on the primary target,
the pixman drain narrowing); the need side is empty.

## Reopen trigger

**A measured client need, benchmarked against the shm path**: a real
client observed allocating (or refusing for lack of) dmabuf capture
buffers, with before/after numbers showing the dma-buf path beats
shm — on a CPU renderer the frame is already in main memory, so
"zero-copy into an encoder" is a hypothesis to measure, not a reason
to build. ScreenCast-through-portal exercised live would be the first
place to look.

## Original ticket (filed 2026-09-19, kept verbatim)

`screencopy::constraints()` returns `BufferConstraints { dma: None, .. }`, so
a capture session offers `wl_shm` buffers only. Filed 2026-09-19, out of
milestone 6 stage 4, which was expected to cover it and should not have: the
two look adjacent and are not the same work.

### Why it is not part of "renderer-derived dmabuf formats"

Stage 4 made the `zwp_linux_dmabuf_v1` tranche derive from what the active
renderer can **import** — a client's buffer coming *in*. Capture is the other
direction, where scoot does the writing, and the capability that governs it is
a different one:

- **It needs a write path that does not exist.** Filling a client's dma-buf
  means `Bind`ing it as a render target and blitting the frame into it.
  Today `deliver` writes through `with_buffer_contents_mut`, i.e. `wl_shm`
  only. That is a new per-capture failure surface on every renderer.
- **The format set is the other one.** What may be offered here is the
  renderer's `dmabuf_render_formats` (what it can render *into*), not the
  `dmabuf_texture_formats` stage 4 derives. On the scanout tier the two are
  already known to differ — `tty/scanout.rs` keeps the render set for
  `DrmCompositor::new` and says in as many words that it never reaches the
  client advertisement.
- **It needs a `DrmNode` the primary target does not have.**
  `DmabufConstraints` requires one. On the GPU-less containers scoot is built
  for there is no DRM node at all, so the honest answer there stays `None`
  whatever else changes — this can only ever be a conditional offer.
- **It trips a documented landmine on the default renderer.**
  `dmabuf.rs`'s `schedule_cache_drain` records that binding a dma-buf render
  target *under pixman* makes every `wl_buffer` destruction in the session
  evict the bound-target cache (upstream's second `retain` drops entries whose
  `dmabuf` is `None`) and force a re-`mmap` on the next frame. The GPU scanout
  tier escapes that because `GlesRenderer::cleanup` retains differently; a
  pixman capture path would not. Narrowing that drain has to land in the same
  change.

### What would justify doing it

A measured client need, which is the same bar `screencopy.rs`'s module doc has
always set for this. Nothing observed so far asks for it: `grim` captures into
`wl_shm`, and quickshell's `ScreencopyView` needs the dmabuf *global* to exist
(which it does — that is what gates its readiness) rather than dmabuf capture
buffers. A screen-*sharing* pipeline that wants zero-copy into an encoder is
the plausible one, and is worth measuring against the shm path before
assuming it is faster: on a CPU renderer the frame is already in main memory.
