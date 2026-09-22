---
item: "6"
title: "Real GPU rendering pipeline"
status: "done"
area: "backend"
pr: 147
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

## Stage 3, part A: the feature gate and the presenter split

Stage 3 is split in two, because the `DrmCompositor` path and the structural
room it needs are not reviewable as one diff. Part A is zero behaviour change
and lands first.

### The spike that decided stage 3 is verifiable here at all

The open question going in (recorded in the stage-2 notes above and in
`HANDOFF.md`) was that **no atomic commit or page flip through a
GBM-allocated framebuffer had ever been attempted on this VM.** The dumb tier
already proves atomic commit, page flip and vblank work here; what was
unknown was whether the primary plane takes a *GBM* framebuffer, and whether
`queue_frame` → `VBlank` → `frame_submitted` round-trips.

A throwaway `crates/scoot/examples/spike_drm_compositor.rs` (deleted after;
it went through `LibSeatSession` exactly as `tty::init` does, so DRM master
was held the same way) answered it, dev VM, working tree at the feature-gate
commit, `2026-09-19T14:02Z`:

```
connector connector::Handle(38) mode (1600, 1000)
surface: crtc=crtc::Handle(37) plane=plane::Handle(33) legacy=false
renderer dmabuf render formats: 80
primary planes kept: 1
DrmCompositor built: format=DrmFourcc(AR24) modifiers=[Invalid]
round 0: commit_pending_before=true is_empty=false
round 0: queued
round 0: vblank(s) [crtc::Handle(37)] after ~50ms
round 0: frame_submitted -> Some(0)
round 1: commit_pending_before=false is_empty=false
round 1: queued
round 1: vblank(s) [crtc::Handle(37)] after ~50ms
round 1: frame_submitted -> Some(1)
round 2: commit_pending_before=false is_empty=false
round 2: queued
round 2: vblank(s) [crtc::Handle(37)] after ~50ms
round 2: frame_submitted -> Some(2)
SPIKE OK: 3 GBM scanout cycles, each confirmed by its own vblank
```

What that establishes, and what it does not:

- **Establishes.** `DrmCompositor::new` succeeds on virtio-gpu with the
  primary plane alone (`cursor`/`overlay` emptied), atomic, at `AR24`. Round
  0 is a modeset (`commit_pending_before=true`), rounds 1–2 are page flips,
  and each one's vblank round-trips its own user data through
  `frame_submitted` — which is exactly the mechanism the session-lock blank
  confirmation has to ride on. `~50ms` is the spike's dispatch granularity,
  not a latency measurement.
- **Does not establish.** `modifiers=[Invalid]` — this device advertises no
  explicit modifiers, so the swapchain is on the implicit/linear path.
  Modifier negotiation on real hardware is untested here. And `is_software()`
  stays false-but-llvmpipe on this VM (see the stage-2 correction above), so
  **real GPU scanout remains an Asahi-only claim**: what the VM proves is the
  KMS plumbing, not the performance case.

### What part A lands

- **`scoot`'s `gpu-scanout` Cargo feature (default off) → `smithay/backend_gbm`.**
  `backend_gbm` is a real link-time dependency on libgbm, unlike `renderer_gl`
  which `dlopen`s libEGL/libGLESv2. GPU-free operation is a fixed decision, so
  the default build must not carry it. Proven rather than asserted, dev VM:

  ```
  $ ldd /tmp/scoot-default | grep -i gbm     # cargo build -p scoot
  (no output)
  $ ldd /tmp/scoot-gpu | grep -i gbm         # + --features gpu-scanout
  libgbm.so.1 => /nix/store/xv6s7zkvnnqjmjfqw09h3099lswrnpyf-mesa-libgbm-26.1.3/lib/libgbm.so.1
  ```

  Off by default does not mean unexercised: the verification set is run twice,
  once per flavour.
- **`tty/dumb.rs`** — the dumb-buffer presenter, moved out of `Tty` whole:
  the `DrmSurface`, the `BufferPool`, the `FlipTracker`, `needs_modeset`,
  `showing`/`pending_free`, `present_skipped`, `retry_armed` and
  `PresentRetries`, plus `present`, `flip_settled`, `next_buffer_age`,
  `advance_generation`, `take_retry_render` and `invalidate_scanout` (which
  came from `hotplug.rs`). Bodies and field docs are unchanged — `git diff
  --color-moved` shows it as a move.

  What stays on `Tty` is what is true of the *session* regardless of how it
  presents: libseat, the `DrmDevice`, the connector and mode being driven,
  `active` (DRM master held) and `session_paused`. `Tty::present` keeps
  exactly its two session-level guards (`!active`, and the frame matching the
  current mode) and delegates the rest.

  This is the room part B needs. Once the presenter is a field rather than
  nine fields spread through `Tty`, a second tier is a second variant, and
  the dumb tier's machinery becomes unreachable from it *by type* rather than
  by everyone remembering not to call it.

## Stage 3, part B: `DrmCompositor` scanout

What lands:

- **`tty/scanout.rs`** -- `ScanoutPresenter`, a `DrmCompositor<GbmAllocator,
  GbmFramebufferExporter, u64, DrmDeviceFd>` plus the bounded retry counter and
  the flip numbering the session-lock wait rides on. **`u64` is the frame's
  `user_data`**: `queue_frame(seq)` -> vblank -> `frame_submitted() ==
  Some(seq)`, so the lock's blank confirmation is matched by the kernel's own
  pairing rather than by "at most one flip is out", which is not true here.
- **`render/scanout.rs`** -- `ScanoutBackend`: the `GlesRenderer`, plus the
  dma-buf a capture reads. The renderer lives in `Backend` because every
  non-frame consumer of a renderer (`capture`, `import_dmabuf`,
  `cleanup_texture_cache`) reaches it only through `Backend`; the
  `DrmCompositor` lives in `Tty` because everything *it* needs is `Tty`'s.
  `render::draw_frame` is the one place holding both.
- **`Presenter::{Dumb, Gpu}`** on `Tty`, and `Pipeline::Scanout` on `Backend`.
  With part A's split in place the dumb tier's `BufferPool`, per-slot ages and
  `FlipTracker` are not merely uncalled on this tier -- they are unreachable
  by type.
- **`--tty --renderer gles`** now selects it, where before `resolve` forced
  pixman. Three fallbacks, all warnings rather than startup errors, because on
  `--tty` a refusal to start is a lockout: no `gpu-scanout` feature in this
  build, no usable GBM node, or no scan-out format that works for both the
  primary plane and the renderer.

### Deliberate scope decisions, each of which could have gone the other way

- **`planes: Some(primary only)`, `gbm: None`.** No cursor plane, no overlay
  planes. *(Superseded by step 1 of
  `docs/backlog/rendering/gpu-scanout-planes.md`: cursor planes now ride
  along with `gbm: Some` where one exists; described here as it landed.)*
- **`FrameFlags::empty()`, not `DEFAULT`.** `DEFAULT` is `ALLOW_SCANOUT`,
  which lets a client's own buffer be scanned out directly on the primary
  plane. That is a real optimisation and it is not this stage's -- with it the
  frame is *not* in the swapchain buffer, so the capture path would silently
  start returning something that is not what is on screen. *(Narrowed by the
  same step 1: the cursor-plane bit alone is now passed; primary/overlay
  direct scanout stays out.)*
- **`PresentRetries` is reused, not replaced**, against the letter of the
  staging note below. A refused `queue_frame` is the same hazard as a refused
  `page_flip`: nothing is in flight, so no completion event will retry it and
  the screen stays stale until unrelated damage arrives. The counter is pure
  bounded-retry arithmetic with no dumb-buffer coupling; duplicating it would
  have been worse engineering than reusing it.
- **A dma-buf import guard, which is not stage 4** *(and which stage 4 then
  removed — see that section below; it is described here as it landed)*.
  `zwp_linux_dmabuf_v1`'s
  tranche is a promise with teeth -- a client that allocates from it and has
  the import refused is *killed*, because `create_immed`'s only failure reply
  is a fatal protocol error. Under `--tty` the renderer has always been
  pixman, which imports a linear dma-buf by mmapping it and essentially never
  refuses; this tier is the first thing that routes those imports through
  GLES, which can. So `ScanoutBackend::new` refuses to come up at all on a
  device whose renderer cannot import what `dmabuf.rs` advertises, and the
  session falls back to pixman. That is not stage 4 (deriving the *tranche*
  from the active renderer); it is the price of adding the tier, per
  `CLAUDE.md`'s rule that a user-facing harm is never deferred to a later
  stage.

### The VT-switch landmine, and why `reset_state` alone is not enough

Smithay's own `DrmOutputManager::activate` is `device.activate()` plus
`compositor.reset_state()`. `reset_state` sets `reset_pending` (so the next
frame is a full commit rather than a page flip onto state another VT may have
reconfigured) but it deliberately does **not** touch `pending_frame` -- and
`queue_frame` only *submits* when `pending_frame` is `None`, queueing behind
it otherwise. A flip still in flight when the session paused, whose vblank the
kernel then never delivers, would therefore leave every subsequent frame
queued and never submitted: a permanently black screen after a switch back,
with no error anywhere. `ScanoutPresenter::reactivate` drains one
`frame_submitted()` after the reset for exactly that, discarding its number
(that frame's content never reached the screen, so it must not confirm a
lock).

`drm_event`'s body is byte-identical to before; only the presenter's own
settle differs per tier. That is what preserves
`docs/backlog/resolved/screencopy-parked-across-lock-confirm-done.md` by
construction rather than by re-tracing it: `note_flip_completed` ->
`confirm_lock` -> `ensure_ticking` still runs on the same event, in the same
order, so a screencopy frame parked across a lock confirmation is still
re-armed.

### Capture without a read-back target

The other two tiers own a persistent framebuffer a capture can be read out of.
Here each frame lands in whichever swapchain slot was free, so the render path
records the dma-buf that carried the most recent *drawn* frame and
`Backend::capture` binds and reads that. Exporting a `Dmabuf` costs an fd per
plane plus allocation, so the exports are pooled per swapchain slot (four
entries; the pool is warm after four frames) and dropped wholesale whenever
the swapchain is rebuilt -- signalled by the presenter through one
`take_slots_dropped` flag, so the only thing that can free a slot is also the
only thing that invalidates the pool.

### Evidence (dev VM, `--tty` seat held 14:40-14:47Z and again 15:11-15:13Z, 2026-09-19)

Everything below was captured twice: first at the tree that became `5ffd3c6`,
then re-run in full at `19f9282` after a comment-and-docs-only commit, so the
key matches the branch head rather than something one commit behind. The seat
was checked free before each session (`pgrep` for another `--tty` client,
`journalctl -u seatd`).

- **The tier really comes up on real KMS**: `drm: driving this device
  path=/dev/dri/card0 connector=Virtual-1 width=1600 height=1000
  scanout="gpu"`, EGL on `PLATFORM_GBM_KHR` over `/dev/dri/card0`, GLES 3.2
  Mesa 26.2.2.
- **The frame is right, and the capture reads the right buffer.** Comparing
  IPC screenshots (ImageMagick `compare -metric AE`, 1600x1000):

  | pair | ImageMagick `AE` |
  | ---- | ---------------- |
  | `--tty` dumb vs `--tty` gpu | 1568.49 |
  | `--headless` pixman vs `--headless` gles | 1568.63 |

  `AE` is ImageMagick's absolute-error metric, **not** a count of differing
  pixels -- review re-ran it and found `differing_fraction=0.999915`, i.e.
  nearly every pixel differs, each by one least-significant bit in one of
  four channels. 1600000/255/4 = 1568.6 analytically, which is what the
  second row is. That makes the conclusion *stronger* than the original
  wording claimed: the two numbers matching says the whole difference is the
  renderer's rounding, and scanout introduces none of its own.
  | `--tty` gpu vs `--headless` gles | 73.96 |

  The first two are the *same* number: the only difference between the tiers
  is the renderer, and it is the pixman-vs-GLES rounding difference stage 2
  already carries (background `srgba(20,20,25)` vs `srgba(20,20,26)`), not
  anything scanout introduced. The third is essentially only the cursor, which
  `--headless` does not draw.
- **A real client end to end**: `MODE=--tty RENDERER=gles
  scripts/smoke-test.sh` -> 17 ok, rc=0.
- **Two VT switch cycles and a hotplug**, on the gpu tier: `session paused` ->
  `session activated` -> full modeset each time, and screenshots taken before
  the first switch, after each switch back and after the hotplug are
  **pixel-identical** (`compare -metric AE` = 0 for all three pairs).
- **Session lock, live, on both tiers** (throwaway
  `examples/tty_lock_probe.rs`, deleted after; a minimal `ext-session-lock-v1`
  client that locks, maps a full-size lock surface, waits for `locked`, and
  unlocks, three times). Every one of the six locks logged `session lock
  confirmed: the blanked frame reached scanout` from inside the `drm_atomic`
  span -- the vblank path -- and the one-second fallback (`confirming a
  session lock without its vblank`) never fired.
- **The default build is unchanged and GPU-free**: `ldd` shows no libgbm,
  `--tty --renderer gles` warns `this build has no gpu scanout tier (it was
  built without the 'gpu-scanout' Cargo feature) ...` and comes up
  `scanout="dumb" renderer=pixman`.

### Benchmark

**Does adding the tier cost the default pixman path anything?** No, and not
measurably in either direction. `crates/scoot/src/compositor/headless/bench.rs`,
release, six rounds run **alternately** against part A's commit (`2ef80d9`,
`git archive`d to `/tmp/scoot-pr1` and built into the same
`CARGO_TARGET_DIR`), because a single pair on this VM says whatever the noise
wants it to say:

| round | base empty | branch empty | base 8win | branch 8win |
| ----- | ---------- | ------------ | --------- | ----------- |
| 1 | 75.87 | 75.08 | 141.82 | 124.43 |
| 2 | 66.16 | 68.11 | 124.81 | 143.61 |
| 3 | 73.08 | 60.73 | 113.82 | 137.33 |
| 4 | 69.52 | 72.30 | 128.06 | 133.05 |
| 5 | 66.86 | 70.97 | 133.43 | 131.32 |
| 6 | 59.63 | 71.83 | 140.36 | 108.59 |
| **median** | **68.2** | **71.4** | **130.7** | **132.2** |

(µs per frame, best of five 500-frame runs each.) The medians differ by 3.2µs
and 1.5µs while each build's own spread across identical rounds is 16µs and
28µs, and the branch holds both the fastest 8-window run (108.6) and the
fastest-but-one empty run. **This VM cannot resolve a difference below roughly
its own ±25% noise, and there is none larger than that here.** Which matches
the code: the pixman arm's work is untouched, `draw_frame`'s per-frame match
gained one jump-table entry, and `Backend::new` gained a startup-only
argument.

**And what does the scanout tier itself cost, here?** More than the dumb tier,
because there is no GPU on this VM -- but far less than the offscreen GLES
pipeline does. Compositor CPU (`utime+stime` from `/proc/<pid>/stat`) over 300
IPC pointer moves on a `--tty` session, two runs each:

| tier | jiffies | wall |
| ---- | ------- | ---- |
| `scanout="dumb"` (pixman) | 27, 24 | 3369ms, 3339ms |
| `scanout="gpu"` (gles) | 36, 37 | 3425ms, 3414ms |

~1.5x, against the **18-31x** stage 2 measured for offscreen GLES on the same
rasteriser. That gap is the read-back and the dumb-buffer memcpy this tier
removes, and it is the only part of the performance story this VM can show.
The part it cannot: what any of this costs on a real GPU, where the
rasterising itself stops being llvmpipe's problem. Do not read these numbers
as a reason to run the tier on a GPU-less box -- pixman is still the right
answer there, and is still the default.

### What this does *not* establish

**Answered on real hardware 2026-09-21 -- see the next section. Kept as
written, because what it was right to disclaim is the point.**

`is_software()` is false-but-llvmpipe on this VM (see the stage-2 correction
above), so **every number here is a software rasteriser's**. What the VM
proves is the KMS plumbing -- modeset, page flip, vblank pairing, VT recovery,
hotplug, lock confirmation -- not the performance case. Real GPU scanout
remains an Asahi-only claim, and the split render/display shape that machine
has (AGX owns the render node, `apple,dcp` owns the CRTCs) is designed for
but untested: `DrmCompositor::new` takes the allocator and the framebuffer
exporter separately, so the allocator's `GbmDevice` would wrap the render
node's fd and the exporter's the display node's, with no `MultiRenderer` and
no speculative multi-GPU abstraction added here.

### Evidence (Apple M2 / AGX under Asahi Linux, 2026-09-21)

The section above is now settled, and one of its hedges turned out to be
unnecessary. Runbook and raw output: `Asahi.md`'s Test 4, driven by
`scripts/asahi-test4.sh`. Two runs -- four alternating rounds, then two more
after the first left two gaps -- on an Apple M2 (`apple,t8112`), NixOS
aarch64, `eDP-1` at `2560x1600@60`, `scale = 1.5`, on battery. Both runs
measured byte-identical binaries (`/nix/store/mf5nm9...-scoot-0.1.0` and
`/nix/store/b1kkl6s...-scoot-gpu-0.1.0`, both `nix build`s of `499083b`),
which is what makes the two runs comparable with each other; the harness
tree differs between them (`650a187`, then `52672b8`) because the harness
was fixed in between, not the compositor.

**The tier comes up, and the split topology needs nothing special.**

```
drm: driving this device path=/dev/dri/card2 connector=eDP-1
     width=2560 height=1600 scanout="gpu"
```

AGX owns `renderD128`; `apple,dcp` owns `card2`'s CRTCs; **one `GbmDevice`
serving allocator, exporter and EGL is enough**. The separable construction
reserved above for exactly this machine is not required, which is the
cheapest possible outcome -- no `MultiRenderer`, no multi-GPU abstraction,
and the code that shipped needs no change. Mode set atomically on
`crtc::Handle(45)`/`plane::Handle(35)`; both tiers logged the same four
warnings, so nothing fell back.

**The frames are right, and the capture reads the right buffer.** On a
pinned scene (two freshly mapped `foot` windows, pointer parked), dumb vs
gpu is identical across all 4.096M pixels except an 18x34 box at physical
1911,1184 with a max channel delta of 3/255 -- the cursor, at
1280,800 x 1.5 = 1920,1200; the crop holds 118 distinct colours and the same
crop elsewhere differs by zero. `AE = 1.003` against the ~4016 (one channel
of four) or ~12044 (all three colour channels, measured) that a whole-frame
one-least-significant-bit difference gives at this resolution. The control:
same tier, different rounds, `AE = 0` exactly, so the scene is genuinely
pinned rather than merely similar. Note the contrast with the VM, where the
whole frame differed by one LSB from pixman's `srgba(20,20,25)` rounding;
here even that is gone and only the cursor's antialiasing differs.

**Performance, per damage event, medians with spread:**

| scene | dumb + pixman | gpu scanout | ratio |
| ----- | ------------- | ----------- | ----- |
| large-damage motion | 0.4553 j/ev (0.4430-0.4613) | **0.0908** (0.0872-0.0972) | **4.8-5.1x** |
| full relayout | 3.358 j/ev (3.32-3.43) | **0.796** (0.790-0.814) | **4.2-4.3x** |

As a share of one core: motion **20.8% -> 4.3%**, relayout **52.0% ->
14.0%**. Per-event normalisation matters -- the tiers get through different
event counts in the same fixed window (156 vs 176 relayouts), so a per-round
total would compare different amounts of work.

**So the extrapolation held, and the reasoning behind it was right.** The VM
measured scanout at ~1.5x *worse* than the dumb tier because there the
rasterising was llvmpipe's problem; remove the read-back on hardware where
rasterising is the GPU's job and the tier wins by 4-5x. What the VM could
not see is exactly what it said it could not see.

**Idle, memory, power:**

- **Idle: 0 jiffies over 10s on both tiers, every round.** Neither wakes when
  nothing moves; idle power is indistinguishable (medians 5.510 W dumb and
  5.505 W gpu whole-system, per-round means spanning 5.411-5.533). The
  fast-but-busy trade does not exist here. Keep that 0.12 W within-tier band
  in view when reading the damage-power figure below: it is over half of it.
- **Memory: +7 to +16 MB RSS** for the GBM swapchain (medians 76.2->92.4 MB
  in run 1, 87.3->94.8 MB in run 2 -- the baseline moves too, so this is a
  range). The one column the dumb tier wins.
- **Power: scanout draws *less*, not more.** Whole-system draw under damage
  is 0.22 W lower under motion (6.26->6.04) and 0.25 W lower under relayout
  (6.36->6.11), about 3.5% of system draw. The "may cut CPU wakeups and raise
  GPU draw, net effect genuinely unknown" question resolves in scanout's
  favour on this hardware.

**Also measured, and it retires the stage-2 caveat too.** The offscreen GLES
tier -- no scanout, `--headless`, so no seat needed -- is at *parity* with
pixman on the same `headless::bench` scenes: 55.0µs vs 56.4µs (empty
desktop) and 57.3µs vs 56.0µs (8 windows), 30 samples each over 6 alternating
rounds, against the **31x and 18x** the same bench measured on llvmpipe.
`GL Renderer: "Apple M2 (G14G B0)"`, GLES 3.2 Mesa 26.2.2, `software=false`
and genuinely so this time. One test per process: the two bench tests
otherwise run concurrently in one process and contend.

Two qualifications on that word "parity", because it is doing more work than
the data supports. **The statistics differ from the ones they sit beside:**
the M2 figures are medians of all 30 per-run values, while the llvmpipe
table above is medians of per-round *minima*. The ratio survives either
choice (median-of-minima on the M2 gives 49.8 vs 54.0µs, still parity), but
the two columns are not the same statistic. **And parity here may be a
floor, not an equality:** adding 8 windows' worth of focus-ring elements
costs pixman +2.3µs on the M2 while it cost *2x* on the VM, which suggests a
fixed per-frame cost dominating both renderers rather than the two genuinely
matching. The safe reading is "the 18-31x penalty is gone", not "the two
renderers are equal".

**One artefact caveat.** Run 1's `summary.tsv` and `environment.txt` were
destroyed after the fact by pointing the entry-point script at its own output
directory (the script now refuses that and offers `ANALYSE_ONLY=1`). The
*file* is no longer re-checkable, but **the rows are**: they were captured
verbatim by review before the loss and are transcribed in
`docs/backlog/resolved/gpu-vs-cpu-measured-done.md` under "Run 1's raw rows,
recovered", where `idle_uW_mean` re-derives from the surviving `.power` files
8-of-8 exactly -- one full column checkable against disk today. Run 2 is
intact and independently carries the correctness result, the ratios and all
the power figures. Run 1's screenshots, power samples and r3-r4 logs survive
(the clobbering run defaulted to `ROUNDS=2`, so rounds 1 and 2 lost their
logs as well).

**What this still does not establish.** The motion scene is *large-bbox*
damage (the injected path jumps across ~900x600 logical pixels), so the
4.8-5.1x is for substantial damage and not for a small cursor-rect move,
which was not measured. One panel, one resolution, one machine. Overlay
planes are still untouched -- but the cursor step has landed: step 1 of
`docs/backlog/rendering/gpu-scanout-planes.md` (cursor plane with graceful
fallback, `ALLOW_CURSOR_PLANE_SCANOUT` only, captures documented as
primary-plane reads) is implemented, including the `CURSOR_PLANE_HOTSPOT`
cap without which paravirt kernels hide the plane entirely. Live on
virtio-gpu it enumerates (`cursor_planes=1`) but the kernel refuses the
atomic TEST, so there it attempts per frame and composites per frame --
active scanout still awaits a real GPU; the `Modifier::Invalid` widening still
has not met a driver that reports `Invalid`-only.

## Stage 4: the dmabuf advertisement follows the renderer

`DMABUF_FORMATS` was a hard-coded `Xrgb8888`/`Argb8888`/`LINEAR` pair, pinned
by a test to what `PixmanRenderer` can import -- and advertised whichever
renderer was active. This stage makes the tranche a *property of the renderer
the session actually built*.

### The ordering problem, and why the global moved

`dmabuf::advertise` ran from `Screencopy::new`, inside `State::new`, **before
any `Backend` exists**: there is no renderer there to ask. Three shapes were
available and two lose.

- **Reorder so the backend exists first inside `State::new`** -- not possible
  without inverting a real dependency, rather than merely inconvenient.
  `Backend::new` needs the `Output`, which needs the framebuffer size, which
  under `--tty` comes out of `tty::init(&mut state, ..)` -- a function that
  needs the `State` (its loop handle, its `renderer`, its `tty` field) to
  exist and that *corrects* `State::renderer` on its way. Moving the backend
  into `State::new` means moving libseat, DRM and udev setup in front of
  `State`, which is a re-architecture, not a reorder.
- **A throwaway renderer at `State::new`, purely to query formats** -- wasteful
  (a second EGL display and GL context) and, worse, *wrong*: it would not
  reliably be the same renderer. `GlesBackend::new` enumerates EGL devices and
  prefers a non-software one; `ScanoutBackend::new` builds on the GBM node
  `tty::init` picked. A probe that landed on a different display would
  describe an EGL context nothing imports through, which is exactly the
  "format table that described a *different* EGL context would be a promise
  nothing keeps" argument `render/scanout.rs` already makes about its own
  formats.
- **Create the global later, once the backend is up** -- what landed.
  `headless::init_named` calls `dmabuf::advertise` immediately after
  `Backend::new`, and `Screencopy::new` keeps a bare `DmabufState` with no
  global.

The risk that shape carries is the one `dmabuf.rs:13-19` records -- this
global gates quickshell's `ScreencopyView` readiness, so a client binding
early must not miss it -- and **it does not apply, because no client can bind
in the window that moved**. The wayland listening socket is a calloop source
(`State::listen`), so no connection is accepted, no registry served and no
global announced until `event_loop.run`. `compositor::run` reaches that
several steps after `init_named` and after the session's own `--` command is
spawned; every test harness is `State::new` -> `headless::init` -> spawn a
client. A global created anywhere in that window was there from the client's
first `wl_registry`.

### What lands

- **`dmabuf::tranche(can_import)`** -- `DMABUF_CANDIDATES` (still
  `Xrgb8888`, `Argb8888`, `LINEAR`, still pinned to `screencopy`'s shm list)
  filtered by `Backend::imports_dmabuf_format`, which is
  `ImportDma::has_dmabuf_format` on whichever renderer the pipeline holds. A
  predicate rather than a renderer, so the decision is unit-testable --
  including the case no machine here can produce on demand, a renderer that
  imports nothing.
- **What counts as evidence is `LINEAR` *or* `Modifier::Invalid`
  (`imports_linear`), and that is load-bearing.** Review caught the first
  version requiring an explicit `LINEAR` entry, which is a false negative on
  real drivers: Smithay inserts `{fourcc, Invalid}` unconditionally and
  explicit modifiers only when `QueryDmaBufModifiersEXT` returned a non-zero
  count (`egl/display.rs:962-1001`), and that count stays zero without the
  modifiers extension, on a driver answering `EGL_BAD_PARAMETER` for its own
  enumerated format (upstream's comment names NVIDIA >= 520, `:938-953`), or
  when a driver reports no modifiers. Such a renderer imports a linear
  dma-buf regardless -- `GlesRenderer::import_dmabuf` never consults the set
  to admit a buffer (`gles/mod.rs:1268-1274`), `Dmabuf::has_modifier()` is
  false for `Linear` so the extension guard does not fire
  (`allocator/dmabuf.rs:229`, `egl/display.rs:754-759`), and no modifier
  attribute is attached (`:817`). The narrow rule would therefore have
  advertised nothing on hardware where the old hard-coded pair worked,
  dropping every GL client to software rendering. The wire still says
  `LINEAR`; only the evidence is wider.
- **Narrowing, not replacing.** The renderer's own set is *not* advertised
  wholesale: GLES on a real driver imports dozens of fourccs, many multi-plane
  or YUV, which no other pixel path here speaks. The direction is asymmetric
  (advertising fewer than can be imported costs a client nothing; advertising
  one that cannot is a `create_immed` kill), so candidates narrow and never
  widen.
- **An empty tranche means no global at all**, not an empty table -- the same
  path a failed feedback build already took. That is the hazard the stage
  exists for: an EGL display with no dma-buf import capability, where the old
  pair would have been advertised and every GL client killed for believing it.
- **`main_device` is the renderer's own DRM render node** where it has one,
  falling back to the `/dev/dri/renderD128` -> `card0` -> `0` path ladder for
  pixman (no device at all) and for a software EGL device (no node at all).
  On a two-GPU machine the path guess can name the *other* card, and a client
  that allocates there hands over a buffer this renderer cannot import.

  Two rungs answer "the renderer's own device", because one of them is not
  enough and that was measured rather than assumed. The offscreen tier's
  display can name its device
  (`EGLDevice::device_for_display` -> `try_get_render_node` -> `dev_id`). The
  **scanout tier's cannot**: its display is made through `PLATFORM_GBM_KHR`,
  and on the dev VM's virtio-gpu that `EGLDevice` carries neither
  `EGL_EXT_device_drm_render_node` nor `EGL_EXT_device_drm` -- the same Mesa
  that answers both for the offscreen tier's enumerated device. So
  `ScanoutBackend` takes the node from the GBM device its EGL display was
  made on instead (`DrmNode::from_file` ->
  `node_with_type(NodeType::Render)`), which is the same device by
  construction. Without that second rung the scanout tier silently kept the
  old path guess, which is the thing this bullet exists to stop.
- **Stage 3B's import guard is removed, deliberately.**
  `ScanoutBackend::new`'s `first_unimportable` computed the identical
  predicate `tranche` now computes and refused to bring the tier up rather
  than narrowing the table. Two mechanisms over one predicate disagree by
  construction; the narrower one is the one that cannot kill a client, so it
  is the one that stayed.

  **The guard was also a net under a false negative in that predicate**, and
  that is the part worth writing down because the first version of this stage
  had one (above): a renderer wrongly judged unable to import lost the tier
  but kept a working dmabuf path, since the session fell back to pixman.
  Without the guard the same false negative ends in no global at all. So the
  removal is correct *conditional on the rule being right*, which is why the
  rule now has its own named function, its own derivation against the pinned
  source, and two unit tests pinning both directions.

  The consequence that remains, stated rather than discovered: on a device
  whose GLES renderer really can import neither candidate, the session comes
  up on the scanout tier with no dmabuf global (GL clients fall back to
  `wl_shm`) where before it fell back to pixman and dumb buffers. No machine
  this project can reach produces that configuration, and `advertise` warns
  loudly -- naming the consequence and `--renderer pixman` as the remedy --
  when it happens.
- **`sync_committed_dmabufs` is now pixman-only.** It is a CPU-cache
  workaround for a renderer that composites out of an `mmap` nothing else
  synchronises; a GLES tier samples the buffer through an `EGLImage`, which
  honours its implicit fences. `handlers.rs` gates it on
  `Backend::maps_dmabufs_on_the_cpu` as well as `State::imports_dmabufs` --
  two conditions asking different questions, deliberately not folded into one
  flag, because `imports_dmabufs`' other reader (the cache drain) is right for
  every renderer.
- **The pinning test became per-renderer**, not deleted: a promise with teeth
  needs *a* test. `every_advertised_format_is_one_the_renderer_imports` reads
  the feedback table off the wire and asserts every entry against the
  session's own backend, which the pixman-only version could not do at all
  under `SCOOT_TEST_RENDERER=gles`.

  **What it does and does not carry, precisely**, because "strictly stronger"
  is too easy to say and this file said it. Under pixman the assertion is
  `for all f in filter(P, C): P(f)` -- true by construction, so what it
  proves *there* is that the derivation survives Smithay's format-table memfd
  and reaches the wire in the right order at the right modifier. The pin
  under pixman is
  the literal `XR24`/`AR24` assertion beside it plus
  `pixmans_own_importable_set_still_contains_both_candidates`, which catches
  a Smithay bump dropping a format from `SUPPORTED_FORMATS` -- something the
  wire test would accept happily by advertising less. Under gles the wire
  test does carry weight of its own, and `!seen.table.is_empty()` is what
  stops it passing vacuously. Not weaker than what it replaced; but it is
  three assertions doing what one used to, not one doing more.

### What is *not* in it

- **`screencopy::constraints().dma` stays `None`.** It is listed with this
  stage in `HANDOFF.md`, and it is a separate feature rather than a line to
  flip: capture is the direction where scoot *writes*, so it means binding the
  client's buffer as a render target and blitting into it -- a different
  renderer capability (`dmabuf_render_formats`, not the import set this stage
  derives), a code path that does not exist, a `DrmNode` the GPU-less
  containers scoot targets do not have, and, under pixman, the bound-target
  cache eviction `dmabuf.rs`'s `schedule_cache_drain` warns about, which would
  have to be fixed in the same change. Filed as
  [`docs/backlog/protocols/screencopy-dmabuf-capture.md`](../backlog/protocols/screencopy-dmabuf-capture.md)
  rather than bundled.
- **The seven `dmabuf::tests` failures under `SCOOT_TEST_RENDERER=gles`**, which
  this stage was twice expected to fix and does not. They are the test
  allocator's `/dev/udmabuf` **provenance**, not format (see the stage-2
  section above); the derived table on that VM names the same two formats the
  hard-coded one did, and the same seven fail identically. A change in that
  count would have been a finding, not a win.

### Evidence

See "Stage 4 evidence" at the end of this file.

## Staging

| Stage | What | Status |
| ----- | ---- | ------ |
| 1 | The renderer seam, pixman the only implementation, zero behaviour change | PR #129, merged `06201e6` |
| 2 | A `GlesRenderer` implementation behind `--renderer`, offscreen + `ExportMem` read-back, so every backend can use it and the existing pixel suites run under both | PR #130, merged `acbdbe0` |
| 3A | The `gpu-scanout` Cargo feature (with the no-libgbm `ldd` proof) and the dumb presenter lifted out of `Tty`; zero behaviour change | PR #133 |
| 3B | `DrmCompositor` scanout for `--tty`: skip the read-back and the dumb-buffer memcpy entirely where a GPU really is present | PR #135, stacked on #133 |
| 4 | Renderer-derived dmabuf formats: advertise what the *active* renderer can import rather than the hard-coded pixman LINEAR pair | PR #147 |

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

## Stage 4 evidence

Dev VM (4 cores, 3.8 GiB, llvmpipe / GLES 3.2 / Mesa 26.2.2, virtio-gpu
`card0` + `renderD128`), `CARGO_TARGET_DIR=/var/cargo-target`,
`CARGO_INCREMENTAL=0`. Everything below was captured at **`e0d6f43`** unless a
block says otherwise; the only commit after it is this section, which is
docs-only.

**An earlier version of this section was keyed to `2b725fc` and is gone
rather than kept**, because review's blocking finding changed `tranche`
itself -- so every number it carried was stale by the project's own cache
rule, and the whole set below was re-run rather than patched.

### The verification set

```
cargo nextest run --workspace
    Summary [ 33.973s] 1108 tests run: 1108 passed, 3 skipped
cargo nextest run --workspace --features gpu-scanout
    Summary [ 33.999s] 1108 tests run: 1108 passed, 3 skipped
SCOOT_TEST_RENDERER=gles cargo nextest run --workspace --no-fail-fast
    Summary [ 49.689s] 1108 tests run: 1101 passed, 7 failed, 3 skipped
cargo test -p scoot
    1003 passed; 0 failed; 3 ignored
       3 passed; 0 failed; 0 ignored
cargo clippy -p scoot --all-targets -- -D warnings                      clean
cargo clippy -p scoot --all-targets --features gpu-scanout -- -D warnings  clean
cargo fmt --check -p scoot                                              clean
```

**Every delta accounted for.** The baseline at `8af76fd` is 1102 passed / 3
skipped in both flavours and 1095 passed / 7 failed / 3 skipped under `gles`.
The whole difference is **+6 tests**, all new:

```
pixmans_own_importable_set_still_contains_both_candidates
a_renderer_that_imports_nothing_is_advertised_as_nothing
a_renderer_missing_one_candidate_advertises_only_the_other
a_renderer_listing_only_the_invalid_modifier_still_advertises_linear
a_renderer_with_only_other_explicit_modifiers_advertises_nothing
the_renderers_own_device_beats_the_path_ladder
```

Four existing tests were renamed rather than added or removed
(`default_feedback_names_a_device_and_both_advertised_formats` ->
`..._and_the_renderers_own_formats`,
`every_advertised_format_is_one_pixman_can_import` ->
`..._is_one_the_renderer_imports`,
`a_session_with_no_renderer_answers_failed` ->
`..._advertises_no_dmabuf_global`, and
`a_non_linear_candidate_is_never_offered` ->
`a_renderer_with_only_other_explicit_modifiers_advertises_nothing`, whose
premise the widened evidence rule changed).

**The `Modifier::Invalid` regression is pinned fail-first, not asserted.**
With `imports_linear` reverted to the narrow rule (`[Modifier::Linear]`
alone) and nothing else changed, at `68d22a2`:

```
cargo nextest run --workspace --no-fail-fast \
  -E 'test(invalid_modifier) or test(only_other_explicit)'

PASS a_renderer_with_only_other_explicit_modifiers_advertises_nothing
FAIL a_renderer_listing_only_the_invalid_modifier_still_advertises_linear
  assertion `left == right` failed
    left: []
   right: [DrmFormat { code: DrmFourcc(XR24), modifier: Linear },
           DrmFormat { code: DrmFourcc(AR24), modifier: Linear }]
Summary: 2 tests run: 1 passed, 1 failed
```

Two things, and the `--no-fail-fast` is there so both are observed rather
than one inferred. `left: []` is exactly the production failure -- an empty
tranche, therefore no global, on a renderer that imports fine. And the
*other* test of the pair passes under the narrow rule too, which is what
says the fix widened the evidence rather than removing it: exactly one test
changes answer between the rules, and it is the one naming the regression.

**The seven `gles` failures are the same seven, by name** -- unchanged, which
is the expected result and not a gap (see "What is *not* in it" above):

```
compositor::dmabuf::tests::a_real_dmabuf_is_imported_and_the_client_gets_its_buffer
compositor::dmabuf::tests::a_rendered_frame_also_drains_the_mapping_cache
compositor::dmabuf::tests::an_accepted_import_claims_and_releases_one_buffer_unit
compositor::dmabuf::tests::an_import_through_create_immed_is_not_a_client_kill
compositor::dmabuf::tests::an_imported_dmabuf_survives_being_recommitted_frame_after_frame
compositor::dmabuf::tests::an_imported_mapping_is_released_without_any_frame
compositor::dmabuf::tests::repeated_import_and_release_without_a_frame_does_not_grow_the_cache
```

### End to end, five backend/renderer combinations

`scripts/smoke-test.sh`, each with its own `SMOKE_PREFIX` so nothing
collides. All five: `rc=0`, **17 ok**.

| run | command | log prefix |
| --- | ------- | ---------- |
| headless, pixman | `SMOKE_PREFIX=/tmp/s4h-hlpx scripts/smoke-test.sh` | `/tmp/s4h-hlpx.*` |
| headless, gles | `RENDERER=gles SMOKE_PREFIX=/tmp/s4h-hlgles ...` | `/tmp/s4h-hlgles.*` |
| nested, pixman | `WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 cage -- env MODE=--nested SMOKE_PREFIX=/tmp/s4h-nest ...` | `/tmp/s4h-nest.*` |
| `--tty`, gles (`--features gpu-scanout`) | `MODE=--tty RENDERER=gles SMOKE_PREFIX=/tmp/s4h-ttyg ...` | `/tmp/s4h-ttyg.*` |
| `--tty`, pixman | `MODE=--tty SMOKE_PREFIX=/tmp/s4h-ttyd ...` | `/tmp/s4h-ttyd.*` |

The two `--tty` runs really were different tiers:

```
drm: driving this device path=/dev/dri/card0 connector=Virtual-1 width=1600 height=1000 scanout="gpu"
drm: driving this device path=/dev/dri/card0 connector=Virtual-1 width=1600 height=1000 scanout="dumb"
```

### `main_device`: every rung, live

The one `info!` line each session logs, from the five runs above:

```
--tty  gles    dmabuf feedback main device device=57984 source="the renderer's own device"
--tty  pixman  dmabuf feedback main device device=57984 source="/dev/dri/renderD128"
headless pixman dmabuf feedback main device device=57984 source="/dev/dri/renderD128"
headless gles   dmabuf feedback main device device=57984 source="the renderer's own device"
nested pixman   dmabuf feedback main device device=57984 source="/dev/dri/renderD128"
```

`57984` is `/dev/dri/renderD128`'s `rdev` and not `card0`'s, which is the
cross-check that matters -- the derived rung really lands on the **render**
node rather than on the primary one the GBM device was opened on:

```
$ stat -c "%n rdev=%t:%T" /dev/dri/renderD128 /dev/dri/card0
/dev/dri/renderD128 rdev=e2:80       # 226:128 -> 57984
/dev/dri/card0      rdev=e2:0        # 226:0   -> 57856
```

So on this machine the derived answer and the old path guess agree, which is
what makes it safe to say the ladder is a fallback rather than a difference
in behaviour here.

**No run logged `can import none of the dma-buf formats this compositor
serves`**, which is the other thing these five sessions establish: this VM's
display lists both candidates at an explicit `LINEAR` among its 76 import
formats, so the widened evidence rule (`imports_linear`) changes nothing
here and the table is the same two formats under either rule. That is why
the widening is pinned by unit test and fail-first rather than by a live
reproduction -- see the verification set above.

### The two findings that changed the diff

**One: the `Modifier::Invalid` false negative** -- found by review, fixed
before merge, and the fail-first proof is in the verification set above.

**Two: the scanout tier's EGL display cannot name its device.**

Captured at `35497f4` (before the fix), `--tty --renderer gles`,
`--features gpu-scanout`, `RUST_LOG=scoot=debug`:

```
scoot::compositor::render::gles: this EGL device cannot name a DRM render node
  error=None of the following EGL extensions is supported by the underlying EGL
  implementation, at least one is required: ["EGL_EXT_device_drm"]
scoot::compositor::dmabuf: dmabuf feedback main device device=57984 source="/dev/dri/renderD128"
```

The scanout tier's EGL display is made through `PLATFORM_GBM_KHR`, and its
`EGLDevice` carries **neither** `EGL_EXT_device_drm_render_node` **nor**
`EGL_EXT_device_drm` -- while the same Mesa answers both for the offscreen
tier's enumerated device, which is why one rung looked like enough and was
not. `ScanoutBackend` now also records the GBM device's own render node, and
the same command at `390ebf6` logs `source="the renderer's own device"` with
the debug line still present on the way past.

### What the commit path costs

**There is no before/after to run here, and manufacturing one would be
dishonest.** The change on that path is the gate, not the work:

```rust
if self.imports_dmabufs
    && self.backend.as_ref().is_some_and(Backend::maps_dmabufs_on_the_cpu)
```

- An **shm-only session** (every test, and any session with no GL client) is
  bit-identical: `imports_dmabufs` is the same `bool` that already gated this,
  and `&&` short-circuits before the second test exists.
- A **pixman dmabuf session** gains one `Option::as_ref` and one discriminant
  compare per commit, both perfectly predicted. That is far below what this VM
  can resolve (its own noise on the frame benchmark is ±25%), and the walk
  behind the gate is unchanged.
- A **GLES dmabuf session** stops doing the walk at all. That saving is the
  only measurable quantity, and it is what `commit_sync_cost` prints:

```
cargo test --release -p scoot --bin scoot commit_sync_cost -- --ignored --nocapture
commit sync, no buffer attached: 52ns per commit (20000 rounds)
commit sync, dmabuf attached:    1.765µs per commit (20000 rounds)
commit sync, gate off: not called -- one bool test in `commit`
```

Two caveats on those numbers, both of which a reviewer needs before comparing
them with the ones in `sync_committed_dmabufs`'s own doc (99ns / 1.39µs):

- **Different codegen.** Run with `CARGO_PROFILE_RELEASE_LTO=false
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, because the repository's release
  profile (`lto = "fat"`, `codegen-units = 1`) **OOM-kills the release *test*
  binary on this VM**: `rustc ... --test -C lto=fat` died with `signal: 9,
  SIGKILL` at ~3.84 GiB of 3.89 GiB used. Stage 3B built that same target
  successfully, so this is an environment change (the binary has grown), not a
  property of this diff -- recorded so the next person does not misattribute
  it. The 32 GiB disk resize that landed on `main` today does not help; this is
  RAM.
- **Same machine, same test, so the shape is comparable even though the
  absolute numbers are not.** What a GLES session now saves is on the order of
  a microsecond per commit per surface carrying a dmabuf, and tens of
  nanoseconds per commit for its neighbours.

The frame path is untouched: `draw_frame`'s per-frame match is unchanged,
`Pipeline`'s size is unchanged in the default build (the new field is on
`ScanoutBackend`, which is boxed and behind `gpu-scanout`), and everything
added runs once at startup.

### What this stage did *not* verify

- **A real dma-buf client, on any tier.** This VM cannot produce one:
  `gbm_bo_create` on its render node is refused, so nothing here can allocate
  a GBM dmabuf at all (recorded in the stage-2 section, unchanged). The
  advertisement is verified on the wire by test and the import path by the
  suite's `/dev/udmabuf` buffers; an end-to-end GL client remains an
  Asahi-only claim.
- **The empty-tranche path live.** No EGL display available here has an empty
  import set, so "renderer imports nothing -> no global" is closed by
  construction and by unit test
  (`a_renderer_that_imports_nothing_is_advertised_as_nothing`), not by a
  reproduction. That is the whole hazard the stage exists for, and saying so
  plainly is better than implying it was reproduced.
- **A multi-GPU machine**, which is where `main_device` naming the renderer's
  own node rather than `renderD128` by path stops being a tidiness argument.
- **A driver that reports its import formats with `Modifier::Invalid` rather
  than an explicit `LINEAR`** -- filed here as unverified when this section
  was first written, then **found by review to be a live bug rather than a
  hypothetical** and fixed before merge (see `imports_linear`, above). What
  stays unverified is only that the *fix* was never exercised on such a
  driver, because no display here is one: the VM's `kms_swrast` reports 76
  formats with explicit modifiers, so both the old rule and the new one keep
  both candidates, and the change is provable against the pinned source plus
  two unit tests rather than reproducible here. The first-run check on real
  hardware is unchanged: `--renderer gles` should log `dmabuf feedback main
  device` and must **not** log `can import none of the dma-buf formats this
  compositor serves`.
