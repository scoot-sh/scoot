---
title: "GPU scanout: cursor + overlay planes (phase 1 landed — the real-GPU proof is in)"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# GPU scanout: cursor + overlay planes (phase 1 landed — the real-GPU proof is in)

Maps to the README bullet clause-by-clause (`README.md:57-65`): (a)
`gpu-scanout`-build-only, (b) primary-plane-only — plus the headless/nested
read-back footnote, which is **design, not TODO** (scanout is tty-only:
headless has no CRTC, nested presents bytes to its host;
`render/gles.rs:15-24`, `docs/tty.md:207-213`). A third clause, "never run
on a real GPU", was the reason this entry had a phase 1 at all; the
2026-09-21 Asahi run retired it and the README no longer carries it. The
bullet now clears on one thing alone: cursor + overlay planes riding KMS
with capture still correct. `scoot-gpu` stays a deliberate opt-in
(link-time libgbm) throughout. Measurement methodology and the numbers live
in [gpu-vs-cpu-measured](../resolved/gpu-vs-cpu-measured-done.md) — this
entry is the correctness work beside it.

## Phase 1 — real-GPU proof on Asahi — **DONE 2026-09-21**

Ran on the user's Apple M2 (`apple,t8112`) under Asahi Linux, `eDP-1` at
`2560x1600@60`. Evidence: `Asahi.md` Test 4's results section and
`docs/roadmap/06-gpu-pipeline.md`'s "Evidence (Apple M2 / AGX under Asahi
Linux, 2026-09-21)"; re-runnable as `scripts/asahi-test4.sh`.

- **It comes up**: `drm: driving this device path=/dev/dri/card2
  connector=eDP-1 width=2560 height=1600 scanout="gpu"`, mode set atomically
  on `crtc::Handle(45)`/`plane::Handle(35)`.
- **The known hazard was not one.** Single-device `DrmCompositor::new`
  succeeds on `apple,dcp` even though AGX owns the render node: **one GBM
  device serving allocator + exporter + EGL is enough**, so the split
  construction reserved at `06-gpu-pipeline.md` is *not* required and
  `render/scanout.rs`'s "expressible later … **Untested**" note needs no
  follow-up. That is the cheapest of the two outcomes this phase was written
  to distinguish.
- **The frames are right**: pinned-scene captures identical to the dumb tier
  across all 4.096M pixels except an 18×34 box at the cursor (max channel
  delta 3/255); same tier across rounds is `AE = 0`.
- **Performance**: 4.2–5.1x less compositor CPU under damage, ~0.2 W less
  power, +7–16 MB RSS, zero idle CPU on both tiers. Numbers, spreads and
  caveats in the roadmap file.

VM baseline for comparison (`06-gpu-pipeline.md:406-502`): KMS plumbing
proven on virtio-gpu; scanout ~1.5x dumb-tier CPU there vs 18–31x for
offscreen GLES — read-back was the dominant cost.

## Phase 2 — overlay + cursor planes (after phase 1 green)

Current locks (`tty/scanout.rs:218-247,539-560`): `planes: Some(primary
only)`, `gbm: None` (cursor plane disabled), `FrameFlags::empty()` (not
`ALLOW_SCANOUT`), `COLOR_FORMATS = [Argb8888, Xrgb8888]`. Order:
**cursor → overlay → direct scanout**, because each widens the capture
contract.

1. **Cursor plane first.** `gbm: Some(...)`, real `cursor_size` for the
   `(64,64)` placeholder (`:242-244`), populate `Planes.cursor`.
   Smallest KMS delta; capture (`render/scanout.rs:21-47`) unaffected.
2. **Overlay planes.** Populate `Planes.overlay` from
   `surface.planes()`; per-CRTC enumeration + fallback where a CRTC lacks
   usable overlays.
3. **`ALLOW_SCANOUT` last.** Deferred at `tty/scanout.rs:207-215` and
   `06-gpu-pipeline.md:343-349` for a reason: the frame may then *not* be
   in the swapchain buffer, so `note_frame`/`frame` capture
   (`render/scanout.rs:92-103,228-293`) silently returns the wrong buffer.
   Lands with a capture fix in the same change, never a flag flip.

Hardware gate: phase 1 is now confirmed, so what remains is a device with
usable cursor/overlay planes; the virtio-gpu VM cannot validate this. The
same Asahi M2 is the candidate -- whether `apple,dcp` exposes usable cursor
and overlay planes at all is the first thing phase 2 has to establish, and
`scripts/asahi-test4.sh` is the harness to extend for it.

## PROGRESS — step 1 (cursor plane) implemented, live proof pending

(branch `cursor-plane-step1`, 2026-09-22; ticket stays OPEN, steps 2-3
remain).

- **The ticket's gate, answered with a surprise.** The dev-VM virtio-gpu
  *does* expose a cursor plane -- `drm_info` on `/dev/dri/card0` shows two
  planes on CRTC 0: Plane 0 (object 33, `type = Primary`) and Plane 1
  (object 34, `type = Cursor`). The "historically has no cursor plane"
  premise was wrong; verified live, per the ticket's own instruction not to
  assume. So the VM can validate the *active* shape, not just the fallback.
- **The Asahi plane inventory is still unknown.** Test 4's evidence names
  the primary plane (`plane::Handle(35)`) but no inventory was ever taken;
  `scripts/asahi-test4.sh` carries no plane enumeration. NOT extended in
  this step, per scope -- the exact commands for the user are filed in the
  PR description.
- **What landed.** `tty/scanout.rs`: `select_planes` (pure: primary narrowed
  to the surface's own plane as before, cursor list rides along whole,
  overlay dropped), `build` takes the device's real `cursor_size`
  (`DrmDevice::cursor_size`, replacing the `(64, 64)` placeholder) and passes
  `gbm: Some` only where the cursor list is non-empty (`None` otherwise --
  the fallback is structural, byte-identical by construction), and
  `render_and_queue` passes `ALLOW_CURSOR_PLANE_SCANOUT` instead of
  `FrameFlags::empty()`. That flag is the whole of step 1's per-frame delta:
  without it Smithay assigns nothing to any plane (traced at the pinned rev,
  `try_assign_element`'s early return), so construction alone would have been
  dead code. It is *not* step 3: `ALLOW_SCANOUT` (primary + overlay direct
  scanout) stays out, pinned by a unit test, with `COLOR_FORMATS` untouched.
- **The capture claim, verified against the pinned source.** The ticket said
  capture was "unaffected" -- it is not, where a cursor plane is active. A
  plane-assigned cursor is never drawn into the swapchain slot, and
  `note_frame`/`frame` (`render/scanout.rs`) record and read exactly that
  slot -- so captures (IPC screenshots, `ext-image-copy-capture-v1`) show the
  screen *without* the cursor on those sessions. Documented where the capture
  lives (`render/scanout.rs` module doc, `screencopy.rs` cursor section,
  `docs/tty.md`), not fixed: compositing the cursor back in would be a second
  cursor render on a path whose point is reading one buffer. `paint_cursors`
  semantics are unchanged in the other direction (captures always contained
  the cursor under `--tty`; now they do except where plane-assigned).
- **Bug-bash, traced.** Cursor-plane claim failure mid-session falls back to
  compositing per frame inside Smithay (`try_assign_cursor_plane` returns
  `None`: no free plane, oversized element, buffer/export failure -- all
  traced, none wedges); cursor size 0/huge is the device's own value passed
  through, with oversize degrading to compositing by the same path; hotplug
  connector switch rebuilds via `adopt_surface`, re-reading the fresh
  surface's cursor list while keeping the device's size; session lock still
  gathers the (reset-to-default) cursor element, so the lock-screen pointer
  rides the plane over the blank in the same atomic commit -- visibility
  unchanged, mechanism only; VT switch pause/resume goes through
  `reset_state` + drain as before, plane state rebuilt on the next frame.
- **Blocked live half.** The dev VM's disk is 100% full (32G: 12G shared
  `/var/cargo-target`, ~10G `/tmp` targets from other agents, 4.4G
  `/var/tmp`), so the `gpu-scanout` build's final link cannot complete there.
  Freed 311M unilaterally (101M own incremental + 211M archived prior-boot
  journals, current boot untouched) -- still short of the ~160M binary plus
  crate-workspace headroom. No feature build, no feature tests, no `--tty`
  proof, no `ldd` yet; nothing of anyone else's touched. Needs ~500M freed
  by the coordinator (or another agent finishing), then: feature build +
  `ldd` libgbm assert, feature unit tests, full gate, `--tty` tier proof
  (`scanout="gpu"` + `cursor_planes=1` + screenshot comparison vs dumb).

## PROGRESS — live half unblocked 2026-09-22: VirtIO hides its cursor plane
without `CURSOR_PLANE_HOTSPOT`, and refuses the TEST when shown

Disk freed by the coordinator; verification completed on the dev VM
(`cursor-plane-step1`, past `cf3b4a4` — see the second commit). Two findings,
one requiring a scope delta, one bounding the outcome:

- **The plane exists but is hidden without a cap the ticket never names.**
  `drm_info`/modetest see plane 34 (`Cursor`), yet `surface.planes()` came
  back `primary=[33], cursor=[], overlay=[]` and the tier logged
  `cursor_planes=0`. Traced with `strace -v`: `SET_CLIENT_CAP{2,1}` and
  `{3,1}` both succeed, yet `GETPLANERESOURCES` returns count=1 on the
  session's fd while a directly-opened fd gets 2. Root cause, from drm-0.14's
  own `ClientCapability` doc: **since kernel 6.x the DRM core hides a
  paravirtualized cursor plane from clients without
  `DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT`** (virtio-gpu, vmwgfx); Smithay's
  `DrmDevice::new` (pinned rev) sets only `UNIVERSAL_PLANES` + `ATOMIC`.
  Proven by experiment: setting the hotspot cap on the live fd flips
  `plane_handles()` from `[33]` to `[33, 34]`. **Scope delta (kept small):**
  `open_device` now sets `CursorPlaneHotspot` once per `--tty` session
  (~8 lines + comment), failing debug-quiet on kernels without the cap
  (which have no hiding to lift). After it the tier logs `cursor_planes=1`.
  The cap's promise (hotspot-managed mouse cursor) is already kept: cursor
  elements arrive hotspot-subtracted and Smithay never writes `HOTSPOT_X/Y`
  (verified absent at the pinned rev), so they stay zero.
- **With the plane visible, virtio refuses the atomic TEST.** Per-frame
  trace (`smithay::backend::drm::compositor=trace`): `trying to render
  element ... on cursor plane::Handle(34)` every frame, then `failed to test
  cursor plane::Handle(34) state`, then the element composites into the
  primary as designed (`drm_info` while running: plane 34 `FB ID: 0`,
  primary 33 flipping). The `using legacy fbadd` warn is Smithay-internal
  (pinned rev, out of scope). So on virtio-gpu step 1 lands as
  **attempt-every-frame + graceful fallback**, not active scanout -- the
  fallback the ticket designed for the no-plane case, exercised harder. The
  capture consequence documented earlier does *not* materialize here (the
  cursor stays composited, captures keep showing it); it awaits hardware
  whose TEST accepts.
- **Numbers (dev VM, llvmpipe -- smoke only, never a GPU verdict).**
  Screenshot matrix, pinned single-foot scene: same-tier control `AE = 0`;
  same-tier cursor move `(800,500)->(200,900)`: `AE = 91.7`, deterministic
  across sessions, localized *exclusively* at the two cursor neighborhoods
  (middle-region crop `AE = 0`) -- the composited cursor moving, nothing
  else. Cross-tier same position (gpu-fallback vs dumb): `AE = 829.6`,
  **max channel delta 1/255, zero pixels above 1%** -- tiers agree to
  rasterizer precision (stronger max-delta than Phase 1's Asahi 3/255).
  Jiffies, real compositor PIDs, single rounds each (no alternation -- a
  smoke, not a benchmark): idle 10s `0` both tiers; 60 large-jump pointer
  moves: dumb `32j` (0.53 j/ev, same ballpark as Asahi dumb 0.455), gpu
  `21j` (0.35 j/ev). VmRSS gpu tier 130 MB. llvmpipe numbers say nothing
  about real GPUs; reported only as no-regression.
- Full gate on the VM: feature build links, `ldd` shows `libgbm.so.1`
  (default build shows zero gbm refs); 5 new tests **execute green**
  (5 passed); `nextest --workspace` **1425 passed, 6 skipped**;
  both `clippy -- -D warnings` clean; `fmt --all --check` clean;
  `smoke-test.sh` `rc=0` (20 oks). One real bug found by the gate: the new
  tests used `smithay::backend::allocator::FormatSet`, which does not exist
  at the pinned rev (it is `allocator::format::FormatSet`) -- fixed; it also
  exposed that the Mac `check`/`clippy` runs never compile this
  Linux-only code at all (smithay is a `target_os=linux` dep), so that
  half of the earlier evidence was vacuous and is corrected here.


## What done looks like

Done for phase 1 (2026-09-21): Asahi numbers recorded in
`06-gpu-pipeline.md`, `Asahi.md` Test 4 appended with results,
`gpu-vs-cpu-measured.md` resolved to
`../resolved/gpu-vs-cpu-measured-done.md`, `docs/tty.md`, `docs/nix.md`,
`README.md` and `ROADMAP.md` no longer say "never run on a real GPU", and
`scoot-gpu` needed no packaging change (the Asahi drivers exposed none).

Still open, and all that is left here: cursor + overlay planes on KMS with
capture still correct, and the README bullet losing its
primary-plane-only clause.
