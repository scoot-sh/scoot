---
item: "6"
title: "Real GPU rendering pipeline"
status: "in-progress"
area: "backend"
pr: 129
commit: null
---

# Real GPU rendering pipeline

A real GPU renderer as an *alternative* to pixman, selected per backend --
never as a replacement. GPU-free operation on webtop and no-GPU boxes stays a
hard requirement (see `CLAUDE.md`'s fixed decisions), so pixman remains the
default and the only mode that needs no device at all.

## Correction: there was no seam to slot into

This file used to end: *"This is exactly why the render-target/presentation
split from item 1 onward matters: it's the seam a GPU renderer slots into."*
**That was false, and it was checked rather than assumed.** Before stage 1
there was no split: `State::render()` was one monolithic ~380-line function
shared by all three backends and hard-wired to pixman in three places --

- `pub struct Backend { renderer: PixmanRenderer, image: Image<'static,
  'static>, damage: OutputDamageTracker, size: (i32, i32) }`
  (`headless.rs:107` at `cb237fb`), with `renderer` and `image` read directly
  by `screenshot.rs`, `screencopy.rs`, `dmabuf.rs` and four test harnesses;
- `render_elements! { Elements<=PixmanRenderer; ... }` (`headless.rs:52`),
  the element enum the whole frame is assembled into;
- the frame hand-off at `headless.rs:614-632`:
  `ExportMem::copy_framebuffer` -> `map_texture` -> `&[u8]` ->
  `Host::present(pixels, w, h)` (`nested.rs:159`) /
  `Tty::present(pixels, region, frame_size)` (`tty/mod.rs:668`).

So building the seam is the work, not a precondition someone already met.
That is stage 1.

## Correction: GLES, not "GLES/Vulkan"

This file used to offer "a GLES/Vulkan Smithay renderer". **The pinned
Smithay rev has no Vulkan renderer at all.** `src/backend/vulkan/` is an
*allocator* (`inner.rs`, `mod.rs`, `phd.rs`, `version.rs`); `rg VulkanRenderer
src/` returns zero hits, and `src/backend/renderer/` carries `gles`, `glow`
(a GLES wrapper), `pixman`, `multigpu` and `test` -- no Vulkan. GLES is the
only GPU renderer available, so that is what the later stages target.

## What a stage-0 spike established on the dev VM

Executed code, not reasoning, and the reason stages 2-4 are believed
buildable at all:

- a `GlesRenderer` **can** be built on the dev VM over software EGL
  (llvmpipe, GLES 3.2, Mesa 26.2.2) and read back with `ExportMem`, so
  stage 2 is testable by running the existing pixel-readback suites under
  both renderers on hardware that has no GPU;
- **the present hand-off needs no change.** With an orientation marker (green
  at the logical top-left, red at the bottom-left), pixman and GLES produce
  byte-identical buffer layouts -- buffer row 0 is the top for both -- so the
  presenters' `Argb8888`/BGRA memcpy and its top-left damage rect stay correct
  under GLES;
- **the trap:** `GlesMapping::flipped()` answers `true` while
  `PixmanMapping::flipped()` answers `false` **for byte-identical layouts**,
  because Smithay composes `flip180` into the GLES projection
  (`gles/mod.rs:2289`). A read-back that "fixed" orientation by honouring
  `flipped()` would undo that compensation and invert the screen. scoot does
  not branch on it, and stage 1 makes that a written contract plus a pixel
  test rather than an accident (see below);
- a generic seam is viable: one `shape_probe<R: Renderer + Bind<T> +
  ExportMem>` compiled and ran against both `PixmanRenderer` and
  `GlesRenderer`.

## Stage 1 (PR #129): the seam, pixman-only, zero behaviour change

What landed:

- **`crates/scoot/src/compositor/render.rs`** -- `Backend` is now
  `{ pipeline: Pipeline, damage: OutputDamageTracker, size: (i32, i32) }`
  with `enum Pipeline { Pixman(PixmanBackend) }`. Damage tracking and the
  framebuffer size are renderer-agnostic, so they live on the outer type and
  a second implementation inherits them instead of repeating them. The
  renderer and its target live in **`render/pixman.rs`**.
- **`render/elements.rs`** -- the renderer-agnostic element gathering
  (cursor / layer / window / ring / lock), lifted out of `render()` and made
  generic. The element enum is
  `render_elements! { Elements<R> where R: ImportAll + ImportMem; ... }` --
  the generic arm of the macro (`element/mod.rs:1693` at the pinned rev),
  the same form `cursor.rs` already used.
- **`render::draw_frame`** is the one place a concrete renderer is chosen: a
  single `match` per *frame*, which is free at 60Hz, handing off to a generic
  `draw_frame_with<R, T> where R: Renderer + ImportAll + ImportMem + Bind<T>
  + ExportMem`. A second renderer is a second match arm and nothing else.
- **`Backend::capture`** wraps the `ExportMem` read-back for its two real
  consumers (`screenshot.rs`, `screencopy.rs`) and the test harnesses.
  Callback-based, so a capture client is still written straight out of the
  renderer's own mapping with no intermediate copy of the screen.
- **`Backend::import_dmabuf` / `cleanup_texture_cache`** replace `dmabuf.rs`
  reaching into `backend.renderer` directly.
- **The `flipped()` trap is now enforced, not just known.** `read_back`'s doc
  states the contract and why honouring `flipped()` would invert the screen,
  and `render/tests.rs`'s
  `the_read_back_hands_out_the_logical_top_row_first` pins it on real pixels
  (proven fail-first: reversing the row order fails it, plus 12 of the
  existing pixel-readback tests).

**Deliberately *not* in stage 1:** no GLES renderer, no `--renderer` flag, no
config key, no dmabuf change. Landing any of those with the seam would make
the diff unreviewable as "provably zero behaviour change", which is stage 1's
entire acceptance bar.

## Staging

| Stage | What | Status |
| ----- | ---- | ------ |
| 1 | The renderer seam, pixman the only implementation, zero behaviour change | PR #129 |
| 2 | A `GlesRenderer` implementation behind `--renderer`, offscreen + `ExportMem` read-back, so every backend can use it and the existing pixel suites run under both | not started |
| 3 | `DrmCompositor` scanout for `--tty`: skip the read-back and the dumb-buffer memcpy entirely where a GPU really is present | not started |
| 4 | Renderer-derived dmabuf formats: advertise what the *active* renderer can import rather than the hard-coded pixman LINEAR pair | not started |

Stage 2 is the first stage with a user-facing surface (`--renderer`), so it
is the first that owes `README.md` a change.
