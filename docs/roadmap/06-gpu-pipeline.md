---
item: "6"
title: "Real GPU rendering pipeline"
status: "in-progress"
area: "backend"
pr: 130
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

## Stage 2 (PR #130): the GLES pipeline, off by default

What landed:

- **`render/gles.rs`** -- `GlesBackend { renderer: GlesRenderer, buffer:
  GlesRenderbuffer }`, built from `EGLDevice::enumerate()` sorted so a
  non-software device is tried before a software one, each candidate tried
  (display -> context -> `GlesRenderer` -> renderbuffer -> a *test bind*)
  until one works. The trailing bind is deliberate: a renderbuffer past the
  driver's `GL_MAX_RENDERBUFFER_SIZE` is created without complaint and only
  fails when attached, which on the frame path would be a per-frame warning
  and a black screen instead of a startup error.
- **The seam took it unchanged.** `draw_frame_with<R, T>`'s bounds
  (`Renderer + ImportAll + ImportMem + Bind<T> + ExportMem`, `TextureId:
  Texture + Send + Clone + 'static`), written for pixman in stage 1,
  type-checked against `GlesRenderer`/`GlesRenderbuffer` with no reshaping --
  the claim stage 1's review made, now executed.
- **`Pipeline::Gles(Box<GlesBackend>)`** -- boxed, because `GlesRenderer` is
  ~6.4KB by value (GL's function-pointer table inline) against
  `PixmanBackend`'s 72, and `State::render` `take`s the whole `Backend` out of
  `State` and puts it back **every frame**. Unboxed, every pixman session
  would pay a 6.4KB memcpy twice a frame for a renderer it never uses.
  (`clippy::large_enum_variant` catches this, and was right.)
- **`--renderer pixman|gles` + `[renderer] backend`**, resolved once by
  `render::resolve(flag, file, tty)`: flag beats file (including
  `--renderer pixman` against a file asking for `gles`, which is why both are
  `Option`), `--tty` overrides both with a warning. Deliberately *not*
  `--gpu`/`[tty] gpu`, which already mean "which DRM device `--tty` drives".
- **Failure is loud.** No EGL device that can drive it is a startup error
  naming each candidate's failure; a config file naming `gles` fails the same
  way (a silent fall back to pixman would make every "verified under GLES"
  claim untrue). `--tty` can never hit that path, so the lockout risk
  `config.rs`'s module doc guards against does not apply.
- **The regression net runs under both.** `SCOOT_TEST_RENDERER=gles` switches
  every `State` the suite builds (the shared `Harness` and the dozen suites
  that still build their own), so the pixel-readback suites *are* the proof.
  `render/tests.rs` adds a renderer-agnostic marker scene asserting pixman and
  GLES produce byte-identical frames, plus the `resolve` precedence tests.

### Evidence (dev VM, llvmpipe / GLES 3.2 / Mesa 26.2.2)

- `cargo test -p scoot`: 995 passed, 0 failed, 3 ignored (984 at `06201e6`,
  +11 new tests). `cargo nextest run --workspace`: 1100 passed (1089 + 11).
- `SCOOT_TEST_RENDERER=gles cargo nextest run --workspace --no-fail-fast`:
  **1093 passed, 7 failed** -- and all seven are `dmabuf::tests`' *import*
  tests. Every pixel-readback suite passes byte-identically under GLES:
  session lock, layer shell, alpha modifier, single-pixel buffer, cursor,
  output scale (including 1.5x/2x, where linear sampling could plausibly have
  differed and does not).
- The chosen device on this VM is `/dev/dri/renderD128`, `software=false` --
  the virtio-gpu render node, which Mesa then serves with `kms_swrast`
  (`GL Renderer: "llvmpipe (LLVM 21.1.8, 128 bits)"`). So the hardware-first
  ordering does prefer a real device node; the software fallback is what a
  box with no node at all would land on.

### The seven dmabuf failures, and why they are *not* stage 4's

Captured with a temporary `tracing_subscriber` in the failing test (reverted;
the compositor never installs one under `cargo test`):

```
[EGL] 0x3003 (BAD_ALLOC) eglCreateImageKHR: createImageFromDmaBufs failed
smithay::backend::egl::display: error=Failed to create `EGLImage` from the buffer
scoot::compositor::dmabuf: dmabuf import refused by the renderer
  error=Failed to convert between dmabuf and EGLImage
  format=DrmFourcc(AR24) modifier=Linear planes=1
```

Not "software EGL has no dmabuf import" -- this display advertises
`EGL_EXT_image_dma_buf_import` and `has_import_dmabuf: true`. The tests
synthesise their dma-buf from a memfd through `/dev/udmabuf`; pixman imports
that by mmapping it, while GLES must hand it to the driver, and `kms_swrast`
refuses a udmabuf-backed import with `EGL_BAD_ALLOC`.

**This was first written up as stage 4's, and that was wrong.** Review
probed the EGL device scoot's own selection picks on that VM and found both
advertised formats (`AR24`, `XR24`) present with `LINEAR` among the
display's **76** import formats — a superset of what is advertised. So the
advertisement is not a broken promise here, and a renderer-derived table
would name the same two formats and fail these seven tests identically.
What `kms_swrast` refuses is the buffer's **provenance**: the tests
synthesise a dma-buf through `/dev/udmabuf`, which pixman mmaps and the GL
path cannot accept. No real client reaches it on that machine either —
`gbm_bo_create` on its render node is refused, so nothing there can produce
a GBM dmabuf at all.

Stage 4 is still worth doing, for a hazard these tests do **not**
demonstrate: an EGL display with no dmabuf-import capability yields an empty
importable set, and advertising pixman's pair against it would disconnect
every dmabuf client. That case is real and unverified. The distinction
matters because "deferred to stage 4" would otherwise read as "already
diagnosed, fix scheduled" when the actual state is "different problem,
unmeasured".

### Benchmarks

Raw numbers in the "Stage 2 benchmark" section at the end of this file. The
shape of them: **pixman is unchanged** by this stage (that is the number that
matters), and GLES on llvmpipe is slower than pixman -- expected, and not a
regression. The performance case for a GPU renderer is real-hardware-only and
is structurally unmeasurable on this VM: there is no GPU here, and stage 2
does not add the scanout path that would be the win even if there were.

### Known, inherited, not fixed here

- **A resize rebuilds the whole GLES renderer** (EGL context, shaders, texture
  cache), because `Backend::new` rebuilds everything and `State::resize_output`
  calls it. Free for pixman, not for GLES. Reshaping that is not stage 2's
  job; `--nested` resizes are rare and a rebuild is correct, just wasteful.
- **Every frame's `Bind<GlesRenderbuffer>` creates and destroys an FBO**, which
  is smithay's design at the pinned rev, not something this seam chose.

## Staging

| Stage | What | Status |
| ----- | ---- | ------ |
| 1 | The renderer seam, pixman the only implementation, zero behaviour change | PR #129, merged `06201e6` |
| 2 | A `GlesRenderer` implementation behind `--renderer`, offscreen + `ExportMem` read-back, so every backend can use it and the existing pixel suites run under both | PR #130 |
| 3 | `DrmCompositor` scanout for `--tty`: skip the read-back and the dumb-buffer memcpy entirely where a GPU really is present | not started |
| 4 | Renderer-derived dmabuf formats: advertise what the *active* renderer can import rather than the hard-coded pixman LINEAR pair | not started |

Stage 2 is the first stage with a user-facing surface (`--renderer`), so it
is the first that owes `README.md` a change.

## Stage 2 benchmark

`crates/scoot/src/compositor/headless/bench.rs`, release, dev VM (4 cores,
3.8 GiB, llvmpipe):

```
cargo test --release -p scoot --bin scoot render_frame_cost -- --ignored --nocapture
SCOOT_TEST_RENDERER=gles cargo test --release -p scoot --bin scoot render_frame_cost -- --ignored --nocapture
```

**Does selecting `gles` cost the default pixman path anything?** No, and the
honest form of that answer is "not measurably here". Baseline is `06201e6`
extracted with `git archive` to `/tmp/scoot-main` and built into the *same*
`CARGO_TARGET_DIR`; the two builds were run **alternately**, eight rounds
each, because a single pair would have said the opposite of the truth (the
first pair happened to catch the baseline's fastest 8-window run and the
branch's slowest, which looked like a 70% regression and was not).

Per-frame minima, µs, in round order:

| Scene | Build | r1 | r2 | r3 | r4 | r5 | r6 | r7 | r8 | median |
| ----- | ----- | -- | -- | -- | -- | -- | -- | -- | -- | ------ |
| empty desktop | `06201e6` | 69.3 | 39.7 | 53.1 | 57.6 | 61.0 | 65.4 | 63.0 | 68.8 | **62.0** |
| empty desktop | branch, pixman | 62.2 | 63.2 | 65.6 | 55.8 | 59.5 | 67.4 | 72.4 | 75.1 | **64.4** |
| 8 windows | `06201e6` | 81.6 | 130.6 | 126.6 | 138.3 | 126.4 | 124.5 | 138.5 | 142.6 | **128.6** |
| 8 windows | branch, pixman | 138.3 | 122.8 | 123.3 | 142.4 | 120.8 | 130.3 | 132.9 | 131.7 | **131.0** |

The medians differ by 2.4µs on both scenes (~4% and ~2%) while each build's
own spread across identical rounds is 19µs and 61µs. The branch has both the
faster 8-window run (120.8 vs 124.5 excluding the baseline's 81.6 outlier)
and the slower empty-desktop tail. **This VM cannot resolve a difference
smaller than roughly its own ±30% noise, and there is no difference here
larger than that.** Which matches the code: the pixman arm's work is
identical, `Pipeline` is the same size (the GLES variant is boxed), and the
per-frame dispatch is the same single match.

**And what does `gles` itself cost?** On llvmpipe, a great deal -- as
expected:

| Scene | pixman (median) | gles (llvmpipe) |
| ----- | --------------- | --------------- |
| empty desktop | 64.4µs | **1.978ms** (31x) |
| 8 windows | 131.0µs | **2.384ms** (18x) |

(Medians across all eight rounds, matching the table above. An earlier
version of this row quoted round 1 — 62.2µs and 138.3µs — which flattered
one scene and penalised the other for no reason beyond round order.)

This is **not a regression and not a reason to tune anything**. There is no
GPU on this VM: `gles` here means Mesa's `kms_swrast` rasterising in software
*and* a GPU-style read-back on top of it, which is the worst of both. The
case for a GPU renderer is a real GPU plus the stage-3 scanout path that
removes the read-back, and neither exists here to measure. Chasing these
numbers would mean optimising for a configuration nobody should run.
