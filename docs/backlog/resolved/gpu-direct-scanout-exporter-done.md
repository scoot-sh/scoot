---
title: "GPU scanout: widen the framebuffer exporter so client buffers can go direct — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GPU scanout: widen the framebuffer exporter — RESOLVED

RESOLVED 2026-09-22 (coordinator-filed, no gh issue; branch
`gpu-direct-scanout-exporter`). All three "What to do" items done; the
headline is a finding the ticket did not anticipate: **widening the exporter
does not make direct scanout reachable, on any hardware**, because the next
gate is the swapchain format match, now its own ticket
([format gate](../core/gpu-primary-direct-format-gate.md)).

- **Filter: `NodeFilter::All`, and why not `Node(..)`** (traced at the
  pinned rev). `can_add_framebuffer` compares the filter against
  `Dmabuf::node()`, and a *client* dma-buf's node is set in exactly one
  place: the client's own `set_sampling_device` (linux-dmabuf v6,
  `wayland/dmabuf/dispatch.rs`). The ticket's premise that the node is the
  feedback's main device was wrong -- the feedback is never stamped onto
  buffers. Mesa's EGL queues (v4) and quickshell (v5) do not even have the
  request, so for them the node is `None`, which `Node(..)` never matches.
  Anvil's `Node(render_node)` works only because its `MultiRenderer` stamps
  every imported buffer (`multigpu/mod.rs:2422`); scoot imports through a
  single `GlesRenderer`, whose EGL *import* never sets a node (only EGL
  export does). `DrmNode` equality also includes the node type, so the
  primary node the GBM device is opened on would not match a render-node
  hint either. The only other reader of `Dmabuf::node()` at the pinned rev
  is `MultiRenderer`, not on this tree, so `All` changes nothing but
  `can_add_framebuffer`. Asahi's split topology needs no special case:
  whatever `render_node()` answers there (the AGX `renderD128`, via the path
  ladder), the exporter imports onto the display card's GBM device and
  refuses cleanly if it cannot.
- **Why `All` cannot hurt a client.** Everything after the filter runs
  against the scanout device and falls back to compositing:
  `framebuffer_from_wayland_buffer` refuses implicit-modifier buffers and
  returns nothing for shm/single-pixel ones, `gbm_bo_import` + `AddFB2` may
  fail, and every failure is a cached `Err` in `element_config` -- nothing on
  that path touches the client's `wl_buffer`, sends an event or posts an
  error. Then the atomic `TEST_ONLY` commit gates any plane. Live: a client
  SIGKILLed while its framebuffer was cached (shipped code) and while its
  buffer was on the primary (experiment) -- compositor alive, capture
  correct (black), primary back on a swapchain fb.
- **What the widening reaches today.** No primary-direct frame (format
  gate), no window on an overlay (no `ScanoutCandidate`), no cursor-plane
  change (Smithay's cursor state has its own `NodeFilter::None` exporter over
  its own buffers). Newly reachable: a client cursor surface whose buffer is
  a dma-buf may ride an overlay where the cursor plane cannot take it (the
  cursorless-capture consequence already documented; virtio has no overlay).
  Measured live on the dev VM (black background, no ring, radius 0, one
  window from a card0 dumb-buffer client): base binary 0 exporter attempts;
  widened binary 2 exports (one per client buffer, then `using cached fb`)
  and 10/10 frames `format doesn't match` -- the widening is effective and
  the format gate is what stops it.
- **Hardware reality on the dev VM, measured with a probe client** (a
  scratch linux-dmabuf client, v4, async `create`, `LINEAR` `XR24`, per
  provenance, against a `--tty --renderer gles` session): card0 dumb
  buffer via PRIME -- **imported**; `gbm_bo_create(LINEAR)` on card0 --
  **imported**; `gbm_bo_create` on `renderD128` -- refused at allocation
  (`CREATE_DUMB: Permission denied`); udmabuf -- refused by the renderer
  (`eglCreateImageKHR: EGL_BAD_ALLOC`, the provenance refusal CLAUDE.md's
  PR #130 note describes, confirmed on this tier too). So something here
  *can* hand the exporter an acceptable buffer, and the exporter does turn
  it into a framebuffer (`AddFB2` succeeds on virtio) -- the format gate is
  the whole reason nothing goes direct, not the hardware.
- **The format gate** (traced + measured). `try_assign_primary_plane` has
  **no element-kind check** (the candidates ticket assumed one); its gate is
  `slot.format() != element_config.properties.format`, whole `Format`.
  The primary exports with `allow_opaque_fallback`, so the client fourcc is
  always opaque (`XR24`) against an `AR24` swapchain (first
  `COLOR_FORMATS` entry), and the client's `LINEAR` modifier is compared
  against, on virtio-gpu, an implicit one (`Testing Formats: [AR24,
  Invalid]`; its primary plane has no `IN_FORMATS`). So even a
  `COLOR_FORMATS` reorder would not match on virtio.
- **Force path proven firing, pinned and live.** Pins drive the code that
  runs: `Captures::capture_target` (the refusal, moved out of
  `Backend::capture`), `Captures::record` (`note_frame`'s body with the key
  and export injected), `tty::scanout::ForceComposite` (arm/take, used by
  `render_and_queue`) -- nine sequence tests including "a forced composite
  whose export fails keeps the mark" (new rule, was implicit) and "the force
  fires for exactly the captures that would refuse". Mutation-checked: each
  of five mutations (serve while marked, clear mark on failed export,
  forced frames keep `ANY`, sticky arming, exporter back to `None`) fails a
  pin. Live, with an uncommitted one-line experiment (`FRAME_FLAGS |=
  ALLOW_PRIMARY_PLANE_SCANOUT_ANY`) and a full-output dumb-buffer client on
  virtio: 10/10 and 24/28 frames assigned to the primary, KMS state showing
  plane 33 on the client's `XR24`/modifier `0x0` 1600x1000 fb; each IPC
  screenshot logged `capture forces a composite frame` and returned the
  client's current shade (single colour 192 or 64, cursor absent on its
  plane); a screenshot while VT-switched away refused with `the current
  frame is held for direct scanout; retry once a composite frame lands`;
  after the switch back it succeeded again. Evidence paths and raw output
  are in the PR description.
- **Two fixes the live run motivated.** (1) Forced frames drop *both*
  primary bits: Smithay tries the primary when the flags intersect either,
  so the old `difference(PRIMARY)` would let a forced frame go direct the
  day `ANY` joins the full set -- exactly the experiment's set. (2)
  `capture_pixels_for` renders *before* forcing (caveat 2): the old order
  let the outstanding render run unforced after the staleness check, so a
  direct client's pending frame could re-mark a just-checked recording and
  fail the capture; the second render it paid was usually an early return
  anyway (`needs_render` cleared by the forced frame). Live: one frame per
  capture on the stale path, the forced composite.
- **No Asahi re-run added.** The format gate applies there too (opaque
  fourcc vs `AR24` swapchain whatever the modifier), so a run could not
  observe direct scanout; `Asahi.md` is unchanged on purpose.

Original entry below, kept verbatim.

---

# GPU scanout: widen the framebuffer exporter so client buffers can go direct

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**
(zero-copy fullscreen video/games on a real GPU) and is the precondition for
[scanout candidates](./gpu-scanout-candidates.md).

## What is missing

`ALLOW_SCANOUT` has been passed since PR #218, and the capture fix that makes
it safe landed with it. But the flag is inert: `tty/scanout.rs`'s `build`
constructs `GbmFramebufferExporter::new(.., NodeFilter::None)`, which rejects
every client dmabuf in `can_add_framebuffer` before any hardware is touched,
and shm/solid elements never produce an exportable buffer. So no frame this
tree produces can take the primary plane directly. `README.md` ("No window
leaves the primary plane yet ... the framebuffer exporter admits no client
buffers") and `docs/tty.md` say so.

## What to do

1. Widen the exporter to admit client dmabufs allocated on the device that
   will scan them out — `NodeFilter` naming the scanout device's own node
   (and, on the Asahi split topology, whatever node the renderer's
   `main_device` advertises to clients — verify what `NodeFilter` variants the
   pinned Smithay rev has and what each compares against, in
   `src/backend/drm/exporter/gbm.rs`, before choosing).
2. **Carry the live force-path verification** the resolved ticket recorded as
   owed (`docs/backlog/resolved/gpu-scanout-planes-done.md`, caveat 1): the
   `ensure_scanout_capture_current` force and the loud refusal have never
   fired. Widening the exporter is exactly what makes them reachable, so this
   change must show them firing — a harness/unit pin at minimum, and live on
   hardware that actually goes direct if any is reachable.
3. Consider caveat 2 while there: `capture_pixels_for` renders twice on the
   stale path; once direct exists, an early return may be worth it (measure).

## Hardware reality

On the dev VM's virtio-gpu (llvmpipe, no render node for clients to allocate
on) it is not obvious anything can produce a client dmabuf that the exporter
would accept — see `CLAUDE.md`'s PR #130 note on `udmabuf` provenance. Go
and measure before either claiming live proof or declaring it unreachable. If
direct scanout cannot be exercised on the VM, the change still has to prove
(by pin + pinned-source trace) that a direct frame cannot produce a stale
capture, and the README must say where it has and has not been seen go
direct. Asahi runs are user-driven (`Asahi.md`); if a re-run is wanted, add
it to that runbook rather than claiming it.

## Out of scope

Marking window elements `ScanoutCandidate` (next ticket). Overlay-plane
assignment of windows. `ALLOW_PRIMARY_PLANE_SCANOUT_ANY`.
