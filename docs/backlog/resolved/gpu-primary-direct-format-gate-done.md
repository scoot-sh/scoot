---
title: "GPU scanout: primary-direct is gated on a swapchain format match no client buffer meets — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# GPU scanout: the primary-direct format gate — RESOLVED

RESOLVED 2026-09-23 (PR #228, branch `primary-direct-fullscreen`), together with the
eligibility half of [candidates](../core/gpu-scanout-candidates.md), which
stays open for overlay-plane candidates and per-surface scanout feedback.
A fullscreen window covering its output, on `--tty --renderer gles` in a
`gpu-scanout` build, whose client submits a dma-buf the display can take,
is now scanned out directly on the primary plane. Everything else
composites as before.

- **`ANY`, per frame, not a format change.** The flag set is decided per
  frame and per output (`tty/scanout.rs`'s `frame_flags`):
  `ALLOW_SCANOUT | ALLOW_PRIMARY_PLANE_SCANOUT_ANY` for a frame
  `render::primary_direct` judges eligible, cursor and overlay bits only
  for every other frame, and a capture-forced frame composites whatever
  its eligibility. That is a behaviour change beyond this ticket's
  `ANY`: the format-matching primary bit used to ride *every* frame (lock
  frames included) and now rides none but the eligible ones. Why `ANY` is
  safe on those, traced at the pinned rev (the constant's doc has the
  whole argument): the client buffer is `AddFB2`'d with its own fourcc
  (and its own modifier where the device takes modifiers; virtio takes
  none, so there the framebuffer is `XR24`/`Invalid` and the driver's
  implicit layout, linear, applies), so KMS never reads it as the
  swapchain's format;
  `try_assign_plane` still refuses unless the plane lists that exact
  format and modifier; the atomic `TEST_ONLY` commit judges scaling, crop
  and transform; and the only real difference, alpha under the opaque
  fallback, cannot show on the bottom plane, which Smithay only hands an
  element that is opaque edge to edge or sits over a black/transparent
  clear colour. Reordering `COLOR_FORMATS` was rejected because it changes
  the format of every composited frame on every device. (Corrected in
  review: on virtio the client framebuffer is `XR24`/`Invalid`, so only the
  fourcc differed from the `AR24`/`Invalid` swapchain there, and a reorder
  *might* have matched -- untried, since `Xrgb8888` rendering, the
  read-back's ARGB assumption and the test commit were never exercised.
  An earlier draft wrongly said it could not match virtio's modifier.)
- **The eligibility rule** (`render/primary_direct.rs`): unlocked; a
  fullscreen window covers the output (`State::covered_by_fullscreen`); no
  capture client is streaming the output; no element in the frame has an
  alpha below 1.0; no element is a rounded window (`Rounded` forwards the
  unclipped buffer as its storage). Judged over the list the frame
  gathered, with the `locked` it was gathered with. Everything above the
  window (an `overlay` surface, a popup, a cursor with no plane) is left to
  Smithay, which only tries the primary for the last visible element once
  everything above it rode a plane -- measured falling back live.
- **Capture streams stay composited.** Measured on the dev VM with the
  first cut (no stream rule), a fullscreen client paced on frame callbacks
  plus a continuous `ext-image-copy-capture-v1` client: every capture
  forced a composite (531-536 forces per run), compositor CPU 805-813
  jiffies/10 s against the composited baseline's 795-805, the client's
  frame interval mean 25.1-25.3 ms against 23.6-23.7 ms (and 461-489
  intervals over 25 ms against 88-104), 531-536 captures in 14 s against
  589-593.
  A net loss, so `Screencopy::streaming` (a session with a frame parked,
  or one that asked within the last second) keeps the output composited;
  with it the same run is 795-804 jiffies against the baseline's 792-798,
  0 forces, 589-592 captures, client interval 23.1-23.2 ms -- the
  composited baseline. IPC screenshots and slow one-shot captures keep the
  force path.
- **Agent screenshot polling (review follow-up).** The first cut reset the
  swapchain for every forced capture (a full GBM reallocation plus
  framebuffer re-registration). A capture behind a direct frame now forces
  a plain composite instead -- Smithay damages the whole output when the
  primary returns from direct scanout (`render_frame`'s
  `had_direct_scan_out` arm), and the damage tracker and swapchain ages
  both count composite frames only -- keeping the reset for an empty
  recording and as a fallback when the plain frame records nothing.
  Measured: IPC `screenshot` polled at 1/5/10 Hz over the direct
  fullscreen client, 2 alternated rounds, against base `3feab9c`:

  | rate | build | CPU jiffies/10 s | latency p50 / p95 ms | client interval mean |
  |---|---|---|---|---|
  | 1 Hz | base | 760, 762 | 12-16 / 17-19 | 23.0 ms |
  | 1 Hz | reset per capture | 54, 59 | 23-24 / 25-28 | 19.4-19.5 ms |
  | 1 Hz | plain composite | 46, 48 | 18-19 / 20-23 | 19.5 ms |
  | 5 Hz | base | 820-826 | 11-16 / 17 | 21.4-21.5 ms |
  | 5 Hz | reset per capture | 205, 206 | 22-23 / 25-27 | 18.2-18.3 ms |
  | 5 Hz | plain composite | 164, 167 | 17-18 / 19-20 | 18.1 ms |
  | 10 Hz | base | 881-890 | 11 / 16 | 20.4-20.5 ms |
  | 10 Hz | reset per capture | 322, 323 | 17 / 20-21 | 17.4 ms |
  | 10 Hz | plain composite | 272, 275 | 14 / 16 | 17.0 ms |

  Counting IPC screenshots toward the stream window was also measured, as
  an uncommitted experiment: it restores base latency (p50 11-14 ms) by
  compositing throughout, and so also restores base CPU (763/822-828/
  892-894), and at 1 Hz it is the worst of both (the output flaps between
  composited and forced). Not taken: it trades ~0.6-0.8 of a core on this
  VM for 3-7 ms of screenshot latency.
- **What it buys, measured (llvmpipe, so the composite side is software
  rendering):** the fullscreen client paced on frame callbacks at default
  config costs the compositor 14-17 jiffies per 10 s direct against
  754-763 composited, and its frame-callback interval drops from 23.2-23.3
  ms to 19.7 ms mean (virtio's vblank). On a real GPU the composite side is
  far cheaper, so expect a much smaller ratio there; the real-GPU check is
  `Asahi.md` Test 5.
- **Live on the dev VM** (virtio-gpu, card0 dumb-buffer client, *default*
  config: ring 3 px, non-black background): the primary scanning out the
  client's framebuffer (Smithay's trace naming the fb, KMS showing it);
  a full-output *tiled* window over black with ring 0 and radius 0 -- the
  PR #222 experiment's shape -- never tested for the primary, with or
  without rounding; captures byte-correct through direct frames (IPC
  screenshot, `grim`, a capture stream); a capture while VT-switched away
  refused with the retry message and served after the switch back; a
  session lock over the direct window composited (0 direct frames between
  lock and unlock, the lock surface captured, direct resumed after
  unlock); leaving/re-entering fullscreen, an `overlay` layer surface
  mapped above, the client switching dma-buf to `wl_shm`, the client
  SIGKILLed on the primary, and an output scale change (the upscaled
  buffer's `TEST_ONLY` fails, it composites, and goes direct again at scale
  1) all fall back and recover. Not reachable there, so not shown: a mode
  change (virtio lists one mode, and wlr-output-management apply is refused
  by design) and a CRTC switch (one connector) -- both rebuild or resize
  through paths that take no part in the per-frame flags.

Original entry below, kept verbatim.

---

# GPU scanout: the primary-direct format gate

Filed 2026-09-22 by the exporter widening
([resolved](../resolved/gpu-direct-scanout-exporter-done.md)), which found this
gate where it expected direct scanout. Serves **daily-drive** (zero-copy
fullscreen video/games). Primary-direct scanout needs **two** things this
tree does not have, on every machine measured (the dev VM; Asahi's
swapchain format was never recorded -- `Asahi.md` Test 5 asks for it):

1. **This gate** -- the swapchain format match, below.
2. **An eligible bottom element.** Smithay only tries the primary for the
   bottom visible element when nothing composited sits above it (the cursor
   must be on its own plane) *and* the element is opaque and covers the whole
   output, or the clear colour is black/transparent. When this was filed
   scoot had no path to that on its defaults: it did not honour client
   fullscreen at all, the default focus ring (3 px) is a composited element
   over the focused window, and the default background is not black. The
   live experiment below needed ring 0, gap 0, one 1.0-wide column and a
   black background to get there. That half is owned by
   [client fullscreen](../resolved/client-fullscreen-done.md) (the state --
   landed 2026-09-22: a covering fullscreen window gets no ring, no rounded
   clip and no `top` layer over it) and
   [candidates](./gpu-scanout-candidates.md) (the eligibility rule) -- both
   must land before a default-config session can go direct.

The one device shape that would already pass this gate: a swapchain that
falls through to `XR24` -- only when the *renderer* cannot render `AR24` or
every `AR24` test commit fails; an `XR24`-only plane is not enough, since
Smithay's opaque fallback still allocates `AR24` for it (the dev VM's primary
is that plane) -- with an explicit `LINEAR` modifier. Covered by the capture
fix if it exists.

## What is missing

Smithay's `try_assign_primary_plane` (pinned rev) has no element-kind check.
Its gate, unless `ALLOW_PRIMARY_PLANE_SCANOUT_ANY` is set, is
`slot.format() != element_config.properties.format` -- a whole-`Format`
comparison, fourcc *and* modifier, between the swapchain slot and the
framebuffer the exporter made from the client buffer. For every buffer a
client can send scoot today it is unequal twice:

- **fourcc**: the primary path exports with `allow_opaque_fallback`, so the
  client framebuffer is always the opaque variant (`XR24`), while the
  swapchain is `AR24` wherever the plane takes it (`COLOR_FORMATS` in
  `tty/scanout.rs` lists `Argb8888` first).
- **modifier**: `zwp_linux_dmabuf_v1` offers only `LINEAR` (`dmabuf.rs`),
  and Smithay refuses to export implicit-modifier client buffers; the
  swapchain on a plane without `IN_FORMATS` is allocated implicitly
  (`Modifier::Invalid`). The dev VM's virtio-gpu is that case (measured:
  `Testing Formats: [AR24, Invalid]`), so reordering `COLOR_FORMATS` alone
  would not match there.

## What is known to work once the gate is lifted

With `FRAME_FLAGS |= ALLOW_PRIMARY_PLANE_SCANOUT_ANY` as an uncommitted
experiment on the dev VM, a full-output card0 dumb-buffer client (card0
allocations, dumb or GBM `LINEAR`, are the only provenance that imports
there -- `renderD128` allocation is refused and `kms_swrast` refuses udmabuf
imports) went primary-direct frame after frame
(KMS plane 33 on the client's `XR24`/`LINEAR` fb), and the PR #218 capture
fix fired live: each capture forced a composite frame and read current
pixels, a capture while VT-switched away refused loudly, and it recovered on
the switch back. A 782x976 window at its column offset failed the atomic `TEST`
(consistent with virtio's primary plane having to cover the CRTC -- not
verified further) and composited, cached as failed. Forced frames
already drop both primary bits (`composite_only`), so adding `ANY` would not
let a forced capture frame go direct.

## What to decide

- **`ANY` vs a format change.** `ANY` skips the format comparison entirely
  and leaves the atomic `TEST` as the only judge -- the simplest lift, and
  the one measured working. Its risk is the reason it is a separate bit:
  it hands KMS buffers whose format differs from the swapchain's, which a
  driver may accept and display with the wrong alpha/colour interpretation
  rather than refuse. Alternatively put `Xrgb8888` first in `COLOR_FORMATS`
  *and* allocate the swapchain with an explicit `LINEAR` where the plane
  allows it -- narrower, but it changes every composited frame's format and
  must be measured (CPU, captures byte-identical, `read_back`'s ARGB
  assumption).
- **Which elements may be tried.** Primary-direct needs the bottom visible
  element with everything above it on planes, and either a black/transparent
  clear colour or a whole-output opaque element. With a black
  `background_color` *any* bottom window qualifies -- and `Rounded` forwards
  `underlying_storage`, so a rounded window taken direct loses its corner
  clip. Decide whether the eligibility rule lives here (e.g. only when the
  element spans the output) or rides the
  [candidates](./gpu-scanout-candidates.md) rule.
- **Cost on the stale-capture path.** Each forced frame costs the direct
  client a framebuffer re-export on its next frame (the composite-only frame
  never runs `element_config` for it, so Smithay's per-element framebuffer
  cache is not carried over: measured 6 exports over a 5-second run with two
  captures vs 2 without) and reallocates the swapchain
  (`invalidate_scanout`). Fine for screenshots; measure before a
  screencopy client at 60 Hz rides it.

## Evidence expected

Live primary-direct on whatever hardware the chosen lift reaches, with
commit health (KMS state naming the client fb), captures byte-correct
through direct frames, and the paused-capture refusal. The experiment
recipe (black background, no ring, radius 0, one full-output column, a
card0 dumb-buffer client) is in the exporter ticket's PR.
