---
title: "GPU scanout: per-surface scanout-tranche dma-buf feedback + presentation zero_copy flag — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GPU scanout: scanout candidates + per-surface scanout feedback — RESOLVED

RESOLVED 2026-09-23 (PR #230, branch `gpu-scanout-feedback`). Everything this ticket
asked for is done except overlay-plane candidates, which were split out
beforehand to [windows on overlay planes](../core/gpu-overlay-window-candidates.md)
(blocked on overlay-capable hardware) and stay open there. The primary-plane
eligibility half landed with the
[format gate](./gpu-primary-direct-format-gate-done.md) (PR #228).

## What landed

- **Per-surface scanout-tranche feedback** (`dmabuf/scanout.rs`), on the
  `--tty` GPU scanout tier only. The root surface of the fullscreen window
  covering an output, while `render::primary_direct` judges the output
  eligible (the judgement shared, not re-derived), is sent per-surface
  `zwp_linux_dmabuf_v1` feedback: a first tranche flagged `scanout`, whose
  `target_device` is the display device (the presenter's DRM fd), then the
  default's own main tranche; same builder, same format table.
- **The tranche is a subset of the advertised default table**, so it
  promises no import the default does not. Per entry, it mirrors what the
  primary plane is asked at the pinned rev: the framebuffer's fourcc is the
  opaque twin (`allow_opaque_fallback`); an explicit modifier must be listed
  by the plane exactly (`IN_FORMATS`); `LINEAR` must be listed explicitly
  where the plane names modifiers for that fourcc, and on a plane naming
  none (virtio) is offered for single-plane packed fourccs -- the
  no-modifier GBM import PR #228 measured going direct; `Invalid` never.
  `get_bpp` was wrongly assumed to mean single-plane (Smithay's table has
  `Nv12`/`Yuv420` at 12 bpp) -- caught by a pinned test, now `bpp % 8 == 0`.
- **Only a frame Smithay would try is eligible (review of PR #230).** The
  first cut steered while `render::primary_direct` said eligible even when
  Smithay's own precondition for trying the primary (`render_frame`: the
  bottom element opaque over and spanning the output, or a black/
  transparent clear colour) could not hold. Review measured it live: an
  `AR24` probe with no opaque region over the default grey background was
  steered twice and never went direct (0 primary assignments). `judge` now
  has a rule 6 (`NothingOpaqueCovers`) mirroring that precondition over the
  frame list, so the direct flags and the steering share it; a frame it
  refuses was never going to be tried, so PR #228's direct frames are
  unchanged.
- **The layout exporter's refusals feed back** (`tty/layout_exporter.rs`,
  `LostLayouts`): a modifier GBM was seen to lose is dropped from the
  tranche and the window re-sent. Static knowledge cannot find that case;
  learning it from the exporter costs nothing where GBM behaves.
- **Churn.** Sent only on a change (Smithay's `set_feedback` also compares).
  The covering window changing reverts at once; the same window staying
  covered but ineligible (lock, capture stream, translucency) reverts only
  on the first drawn frame once that has lasted 2 s (`REVERT_HOLD`, longer
  than the 1 s stream window, so a thumbnail capture never makes a game
  reallocate; frame-driven, no timer, so a screen that stops drawing keeps
  the scanout feedback, harmlessly); an overlay surface or
  popup above the window is not an eligibility rule and reverts nothing
  (a deliberate deviation from the dispatch's "overlay above" revert case:
  it is transient, and the window goes direct again in the steered layout
  once it is gone). Built once per plane set (key: CRTC switch epoch +
  lost-modifier generation), cached including a failure; per frame one key
  compare and the tracker's comparisons, no allocation.
  `new_surface_feedback` answers a surface that first asks after going
  fullscreen with the scanout feedback.
- **`wp_presentation` `zero_copy`** for the surface whose buffer the primary
  plane scanned out that frame, from the `DrmCompositor`'s own answer
  (`PrimaryPlaneElement::Element`'s id, carried as `ScanoutFrame::primary_direct`
  -- the same field the capture mark reads, now an `Option<Id>`). Not
  Smithay's `surface_presentation_feedback_flags_from_states`, which marks
  the cursor plane `ZeroCopy` too. A client cursor on an overlay plane is
  not reported (under-reported, never misreported).

## Evidence (summary; commands, SHAs and raw paths are in the PR)

- Harness: `dmabuf/scanout/tests.rs` (the tranche rule per plane shape,
  an ordered-subset sweep over 128 plane shapes, single-plane predicate),
  `fullscreen/tests/scanout_feedback.rs` (over the wire: sent once, nothing
  per frame, immediate revert on unfullscreen/unmap/another window, lock
  held then reverted, translucent moment shorter than the hold sends
  nothing, overlay above keeps it, late request answered with scanout,
  empty tranche steers nothing, rebuilt tranche re-sent, every pair in the
  default and importable), `screencopy/tests/streaming.rs` (a stream holds
  then reverts; a one-shot capture does not flap),
  `presentation_time/tests.rs` (`zero_copy` exactly on the direct surface),
  `tty/layout_exporter/tests.rs` (lost-modifier record).
- Live on the dev VM's `--tty` scanout tier (virtio-gpu, card0 dumb-buffer
  probe binding v4 surface feedback and `wp_presentation`): tranche
  `XR24`/`AR24` at `LINEAR`, `target_device` card0 (`0xe200`) vs
  `main_device` renderD128 (`0xe280`); sent on fullscreen, reverted on
  unfullscreen and on the first frame 2.0 s into a lock or stream (the
  probe commits on a timer, so frames kept coming), not touched by an overlay
  surface or a `grim` one-shot; the probe reallocating on every feedback
  kept going direct (KMS primary fb = the traced direct fb, screenshots
  correct); `zero_copy` presented exactly as often as Smithay assigned the
  primary (260/260 and 384/384). Compositor CPU with a direct fullscreen
  client unchanged against `main` (13-16 vs 14-16 jiffies/10 s).
- Real GPU: whether a GL client reallocates into the tranche and goes
  direct is `Asahi.md` Test 6 Part C -- not claimed. No GL client on the
  dev VM can allocate a dma-buf at all.

Original entry below, kept verbatim.

---


# GPU scanout: scanout candidates + per-surface scanout feedback

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**.

**Update 2026-09-23: the primary-plane half is done** -- see the
[format gate](../resolved/gpu-primary-direct-format-gate-done.md). A
fullscreen window covering its output now goes primary-direct, decided per
frame by `render::primary_direct` (unlocked, covered, no capture stream, no
translucent or rounded element), with `ALLOW_PRIMARY_PLANE_SCANOUT_ANY` on
eligible frames only. That settled the eligibility rule this ticket asked
for *for the primary*. What is left here is what it scoped out:

- **Overlay planes** — split out 2026-09-23 to
  [gpu-overlay-window-candidates](../core/gpu-overlay-window-candidates.md)
  (blocked on hardware with overlays). Original note: no window is `Kind::ScanoutCandidate`, so none rides an
  overlay. The rule below is still the one to adopt for that; the rounded
  and translucent refusals already exist in `render::primary_direct` and
  should be shared, not copied. Before marking anything, extend the capture
  contract: `Captures::note_direct` fires only for primary-direct today
  (`ScanoutFrame::primary_direct`); a window on an overlay is equally
  absent from the swapchain slot. Virtio has no overlay plane, so this
  needs hardware that has one.
- ~~Depends on gles-dmabuf-full-formats~~ — landed 2026-09-23
  ([resolved](../resolved/gles-dmabuf-full-formats-done.md)): the GLES
  default tranche is the driver's real set.
- **Per-surface scanout-tranche dma-buf feedback**, so a client can
  allocate a buffer the plane takes. More pressing now than when filed: the
  default tranche used to offer `LINEAR` only, which happened to be
  scannable on the machines measured; under `gles` it now offers the
  driver's tiled/compressed layouts too, and a fullscreen client that picks
  one the display cannot scan out composites instead of going
  primary-direct. (On the dev VM nothing changes — llvmpipe lists only
  `LINEAR`. What AGX/DCP do is `Asahi.md` Test 6.)
- **Presentation feedback's `zero_copy` flag** for a surface whose buffer
  went direct (informational; `wp_presentation` flags stay `vsync`-only
  today, which under-reports rather than misleads). Smithay's
  `RenderElementStates` already carries `ZeroCopy` per element.

The [exporter widening](../resolved/gpu-direct-scanout-exporter-done.md) it
depended on has landed (`NodeFilter::All`), and so has
[client fullscreen](../resolved/client-fullscreen-done.md), which supplies the
whole-output window state the eligibility rule below needs:
`World::fullscreen_on(output)` (which window covers an output right now) and
`Placement::fullscreen`. Under a covering window the compositor already draws
no ring, no rounded clip and no `top` layer over it (`overlay` surfaces and
the cursor may still be above it).

## What is missing

Every window surface element is built `Kind::Unspecified`
(`render/elements.rs`, both call sites; `decorations.rs`), only cursor
elements are `Kind::Cursor`, and `Rounded` forwards its inner kind.
Smithay's **overlay** assignment only considers `ScanoutCandidate` (and
`Cursor`) elements, so no window can ride an overlay plane.

Primary-direct is gated differently: `try_assign_primary_plane` has no kind
check at all. It is tried only for the bottom visible element with nothing
composited above it, that element opaque and covering the whole output (or
a black/transparent clear colour), and then requires the client
framebuffer's whole `Format` to equal the swapchain's
([format gate](../resolved/gpu-primary-direct-format-gate-done.md) -- now
skipped with `ANY` on eligible frames). On default config no
window meets the first half unless it is fullscreen (see
[client fullscreen](../resolved/client-fullscreen-done.md)): otherwise the
3 px focus ring is composited over the focused window, and the background is
not black. The
eligibility rule here is therefore also what decides which windows may be
*tried* for the primary -- one decision, shared with the format-gate ticket
(made for the primary on 2026-09-23; see the update above).

On the rounded clip: `Rounded` forwards `underlying_storage` (and `kind`),
so the element Smithay would scan out is the unclipped inner buffer -- the
corners are lost, not approximated.

And a client cannot know which buffers would be scanout-able: the
`zwp_linux_dmabuf_v1` feedback today has one renderer tranche. Smithay's
pattern (anvil at the pinned rev) is per-surface feedback with a
scanout tranche built from the plane's formats, sent when a surface becomes
a candidate.

## What to do

- Decide the eligibility rule and write it down: the obvious first cut is a
  window that covers the whole output (fullscreen, from
  [client fullscreen](../resolved/client-fullscreen-done.md), or a sole full-size column),
  with nothing composited over it (the focus ring hidden or not drawn over a
  fullscreen window),
  unrounded (a rounded window's clip means it cannot be scanned out whole —
  `corner_radius > 0` must exclude it unless the radius clip is provably a
  no-op), not under any other element, alpha 1.0, no transform/viewport the
  plane cannot express. Anything else stays `Unspecified`.
- Mark those `Kind::ScanoutCandidate`; everything that relies on captures
  reading the swapchain slot is already covered by the PR #218 force path —
  verify it still is once *windows*, not just the primary, can go direct
  (a window on an overlay plane is also absent from the swapchain slot:
  confirm `note_direct` covers overlay assignment of a window, not only
  primary-direct, and fix if not — that is a correctness blocker, not a
  follow-up).
- Per-surface dmabuf feedback with a scanout tranche for candidate
  surfaces, falling back to the default feedback when a surface stops
  being a candidate.

## Evidence expected

Live on whatever hardware goes direct (see the exporter ticket's hardware
note); if none is reachable from here, harness pins plus a clearly worded
README limitation. Captures must stay byte-correct with a candidate window
on screen — that is the property the whole plane series has protected.
