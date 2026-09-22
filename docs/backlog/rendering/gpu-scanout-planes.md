---
title: "GPU scanout: cursor + overlay planes (steps 1-2 landed — the real-GPU proof is in)"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# GPU scanout: cursor + overlay planes (steps 1-2 landed — the real-GPU proof is in)

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

> **PARTLY SUPERSEDED by the review follow-up below.** The hotspot root
> cause stands; the "virtio refuses the atomic TEST" conclusion does not --
> the refusal was the stale prop-mapping snapshot (post-cap placement), and
> with the cap moved pre-`new` the TEST passes, commits succeed, and the
> plane is ACTIVE. The fallback-behavior numbers below remain valid as
> fallback characterization, but they describe the buggy head, not the
> fixed one.

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

## PROGRESS — review follow-up: the post-cap position caused UnknownPlane
black screens; atomic-first pre-`new` placement makes the plane ACTIVE

`scoot-reviewer` blocked: `AtomicDrmDevice::new` snapshots
`plane_handles()` into its property mapping once at construction, so the
post-`DrmDevice::new` cap revealed plane 34 to `surface.planes()` (fresh
query) but not to the mapping -- every commit failed `UnknownPlane(34)`,
primary never flipped, black screen. The reviewer's live `drm_info`
contradicted the earlier "primary flipping" claim (FB 0 on both planes);
the post-cap jiffies comparison is void (non-presenting vs presenting),
and screenshots were structurally blind to it (they read the swapchain,
which renders fine either way) -- commit-health assertions
(FB-flipping/UnknownPlane-absence) are now required per live claim on this
ticket. Verified the mechanism against the pinned source (`device/atomic.rs`
snapshots once) and moved the cap to between `DrmDeviceFd::new` and
`DrmDevice::new` -- with one correction to the reviewer's prescription:
`CURSOR_PLANE_HOTSPOT` needs `ATOMIC` set first (measured `EINVAL`
otherwise, matching drm-rs's own doc), so the code sets `ATOMIC` then
`HOTSPOT` pre-`new`; the `ATOMIC` re-set inside Smithay's `::new` is idempotent
(Smithay sets `UNIVERSAL_PLANES` + `ATOMIC`, never `HOTSPOT` — that one is set
once, by scoot; strace-verified `{3,1}=0,{6,1}=0` then `{2,1}=0,{3,1}=0`).

End-state on virtio-gpu is now **plane-ACTIVE, not fallback**: `UnknownPlane`
count 0, plane 34 shows `FB ID: 46` on CRTC 37, and `CRTC_X/Y` track the
pointer across moves (image top-left = pointer − the live cursor's hotspot;
e.g. `(192,892)` for pointer `(200,900)` with the 8px-hotspot cursor that
was live then — the default arrow uses hotspot `(0,0)` and tracks 1:1). Screenshots across a move are **byte-identical
(`AE = 0`)** -- the cursor is on the plane, invisible to capture, exactly the
documented consequence, now live-observed with before/after crops (cursorless
vs dumb-tier cursor-ful at the same coords). Notably the earlier TEST refusal
(`failed to test cursor plane state`) is GONE with the snapshot fixed -- it
was the same root cause, not a driver limitation; zero occurrences on the
fixed head. The per-frame `info!` dampening (`EnvFilter` default demoting
`smithay::backend::drm::compositor` to `warn` -- its sole `info!` at the
pinned rev is that line) is retained as defense-in-depth for other hardware.

A/B jiffies, alternating sessions G,D,G,D,G,D, 60 large-jump events each,
real PIDs: gpu `[21, 15, 14]` (median 15, 0.25 j/ev), dumb `[32, 31, 30]`
(median 31, 0.52 j/ev) -- ranges do not overlap; llvmpipe smoke only, no GPU
verdict. Measurement-hygiene notes: an early single-round comparison is
superseded by this A/B; a mid-session PID mixup (a lingering `bash -c`
launcher wrapper measured at 0j) was caught by checking `comm` and redone;
cargo-over-9p sometimes reports no-op `Finished` where a rebuild was
expected -- attested binaries since via `strings`+`ldd`+behavior, and forced
relinks where it mattered.


## PROGRESS — step 2 (overlay planes) implemented + live-proven on virtio 2026-09-22

(branch `overlay-planes-step2`; ticket stays OPEN, step 3 remains).

- **Overlay inventory, virtio-gpu: zero, verified live.** `drm_info` on
  `/dev/dri/card0` shows two planes on the single CRTC: object 33
  (`Primary`) and object 34 (`Cursor`) — no `Overlay` type anywhere. The
  ticket's "virtio historically exposes no overlay planes" holds on this
  machine. So the VM validates the *fallback* shape live (enumerated zero,
  behaved byte-identically), not active overlay scanout.
- **Asahi `apple,dcp` overlay inventory: still unknown.** Same gap as the
  cursor had. NOT extending `scripts/asahi-test4.sh` per scope — the exact
  user commands are filed here for the coordinator to run on the hardware:
  ```sh
  drm_info | grep -E '"type"|CRTCs|Object ID'   # one-line per-plane inventory
  drm_info | sed -n '/Planes/,$p'              # full per-plane dump (formats, FB, CRTC_X/Y)
  ```
  What to look for: any plane with `"type" = Overlay`, which CRTCs it is on,
  and (while the gpu tier runs) whether its `FB ID` goes non-zero.
- **What landed.** `tty/scanout.rs`: `select_planes` passes the overlay list
  through whole (`inventory.overlay.clone()`); `PLANE_FRAME_FLAGS`
  (renamed from `CURSOR_FRAME_FLAGS`) is cursor bit + overlay bit;
  `ScanoutPresenter` gains `overlay_planes` (set in `new`, re-read in
  `adopt_surface` with the re-emit log extended to both counts);
  `render_and_queue` passes the widened flags; the startup line is now
  `drm: scanout cursor planes cursor_planes=N overlay_planes=M ...`
  (message kept grep-stable, field added). `render/scanout.rs` + `screencopy.rs`
  capture docs extended to the overlay shape (below).
- **The whole-vs-narrowed decision, traced not assumed.** Overlay rides
  whole like the cursor, NOT narrowed like the primary, for three pinned-rev
  reasons: (1) there is no per-plane choice to make at construction —
  Smithay claims one plane per frame from the handed-over lists and falls
  back to compositing wherever none can be claimed (oversized element,
  failed TEST, occupied plane — all `trace!`, none wedging the frame);
  (2) `DrmCompositor::new` sorts the overlay list front-to-back itself, so
  narrowing would only risk dropping a plane it could have used; (3)
  construction cannot newly fail because of it — the swapchain format search
  (`find_supported_format`) reads the *primary* plane's formats only, so an
  overlay-only format gap can refuse one frame's assignment but never the
  compositor's construction. Per-CRTC enumeration + mixed fallback come free:
  every surface carries its own CRTC's inventory, `adopt_surface` re-reads
  the fresh one, and the empty-overlay CRTC selects the empty list (the old
  construction behaving exactly).
- **The capture answer (load-bearing): at most the cursor is ever missing —
  document, no escalation, no composite-fix.** Smithay's
  `try_assign_overlay_plane` only considers elements of kind
  `ScanoutCandidate` or `Cursor`, and this tree builds *every* window, popup
  and layer-shell surface as `Kind::Unspecified` (both
  `render_elements_from_surface_tree` call sites in `render/elements.rs`;
  `Rounded` forwards its inner kind unchanged) — only cursor elements are
  `Kind::Cursor`, and the cursor plane is tried before the overlay. So no
  window can ride an overlay plane on any hardware until something is marked
  a scanout candidate, and that marking belongs to step 3 *with* its capture
  fix: marking one now would let whole windows leave the buffer captures
  read, which is exactly the misleading-capture harm this ticket says to
  escalate on rather than document. What the overlay bit does today is let
  the cursor ride an overlay where a CRTC has overlays but no cursor plane —
  the same cursorless-capture consequence step 1 already documents, widened
  by one plane kind. Measured live (virtio, cursor plane active, zero
  overlays): captures across a cursor move byte-identical (`AE = 0`, same
  md5); cross-tier same-position diff 281 px raw / 177 px at 5% fuzz out of
  1.6M (0.018%), all inside the cursor's third (middle/right thirds `AE =
  0`); same-tier dumb move footprint ~77 px. Nothing window-sized is missing.
- **No new log spam: EnvFilter untouched.** The only `info!` in Smithay's
  `drm-compositor` target at the pinned rev is still the cursor TEST-fail
  line step 1 dampened (verified by grep — one hit); every overlay
  assignment failure path (`try_assign_plane` and callers) logs at `trace!`.
  The existing `info,smithay::backend::drm::compositor=warn` default covers
  both with nothing to extend and nothing duplicated.
- **Explicitly still out, pinned again:** `ALLOW_SCANOUT` (both primary
  bits — the flags test asserts their absence alongside `!= ALLOW_SCANOUT`
  and `!= DEFAULT`), `COLOR_FORMATS` expansion (untouched), per-output
  scale/mode, packaging. No `PROTOCOL_VERSION` bump: no IPC shape, request,
  action or reply field was added or changed — the diff is KMS plane
  assignment plus log fields, none of which crosses the socket.
- **README bullet: verified, no clause clears — step 3 still owns it.**
  Overlay-active scanout of windows does not exist yet (nothing is a
  candidate), so the "primary-plane-only" clause still describes what runs.
  Flagged, not fixed: the bullet's cursor half ("no overlay or cursor planes
  yet") went stale under step 1 (PR #216 attempted the cursor plane without
  touching the line) — left for step 3's bullet rewrite per this ticket's
  scope, stated here so it is not mistaken for step-2 drift.
- **Bug-bash, live where reachable.** Overlay attach failure mid-session:
  unreachable on virtio (no planes to attach); the refusal path is Smithay's
  per-frame TEST→`Err`→composite, unchanged in shape from the cursor's and
  observed at zero occurrences (`failed to test` count 0, `UnknownPlane`
  count 0). COMMIT failure / lock / VT / hotplug with overlay state: with
  `overlay_planes=0` there is no overlay state to disturb — `reactivate`,
  `invalidate_scanout` and `use_mode` do not branch on the counts, and
  `adopt_surface`'s only new branch is the log condition (single CRTC here,
  so the re-emit is code-read plus the mixed-inventory unit pin, same bar
  step 1's fix was held to). Zero-overlay CRTC alongside an overlay CRTC:
  each `adopt_surface` re-reads the fresh surface's own inventory — pinned by
  the mixed unit test (cursor-empty/overlay-full selects exactly that).
- **Numbers (dev VM, llvmpipe — smoke only, never a GPU verdict).**
  Full gate at the branch head: feature build links, `ldd` pair 1 vs 0 gbm
  refs (default binary additionally `strings`-clean of the new log line);
  7 scanout unit tests green (3 new: ride-whole, empty-fallback, mixed);
  `nextest --workspace` **1425 passed, 6 skipped**; feature-package nextest
  **1270 passed, 6 skipped**; both clippys `-D warnings` clean;
  `fmt --all --check` clean; `smoke-test.sh` rc=0, 20 oks.
  Live `--tty` (seat checked free, `/tmp/scoot-overlay2` prefix, released
  after): `scanout="gpu"`, `cursor_planes=1 overlay_planes=0`; cursor plane
  FB 46 active with pointer-tracked CRTC_X/Y; primary FB 42→45 across a
  spawn (commits healthy, flips on damage, still on damage-only);
  alternating A/B G,D,G,D,G,D × 60 large jumps, real PIDs (`comm=scoot`
  checked): gpu `[5,5,5]`, dumb `[6,9,6]`, idle 0–1j both tiers, RSS
  ~123.7MB vs ~42.4MB. Smaller absolutes than step 1's A/B (tighter,
  sleep-free move loop lets damage coalesce) — same method both tiers, gpu
  no worse in all three pairs; reported only as no-regression.


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
