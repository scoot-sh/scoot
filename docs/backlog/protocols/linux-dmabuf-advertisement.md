---
title: "Advertise a minimal, honest `zwp_linux_dmabuf_v1` so quickshell's `ScreencopyView` leaves the readiness gate."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Advertise a minimal, honest `zwp_linux_dmabuf_v1`.

Filed from the measurement that closed the gating question of
[`screencopy-shell-thumbnails-fallback.md`](screencopy-shell-thumbnails-fallback.md)
as YES on 2026-09-17: advertising the global flips quickshell 0.3.1's
`WlBufferManager::isReady`, and the already-shipped ext output-capture path
then displays over shm — with an advertisement whose every datum is true but
whose import-capability implicature is overstated (that file's one-sentence
honesty verdict). This item is the small advertisement follow-up the ticket
names: no capture work, no protocol work beyond the one global.

## What to build

Essentially the probe, productionised: Smithay's `DmabufState` +
`DmabufHandler` at the pinned rev, `create_global_with_default_feedback`,
feedback = `main_device` (this machine's real scanout `dev_t`; `0` where no
DRM node exists, logged) + the formats the shm pipeline actually serves
(`Xrgb8888`, `Argb8888`) × `LINEAR`, and `dmabuf_imported` answering
`failed` — the protocol's own "cannot import for implementation-dependent
reasons", which is the only truthful answer a pixman/shm compositor has.
The probe diff below is the starting point, not the shape: it needs the
`PROBE` markings and `warn!` scaffolding replaced with the module's real
logging voice, a home inside `screencopy.rs` (or beside it, if it sprawls),
tests for the parts that are testable without a GPU (global advertised,
feedback well-formed, `create_params` answered `failed` rather than hung or
crashed), and the same trust-model note `screencopy.rs` already carries for
its capture globals.

## Acceptance (measured, not reasoned)

- Re-drive `/var/tmp/qs-toplevel-probe.sh` against the shipped tree:
  `outview hasContent=true`, one surviving `no recording context` (the
  `Toplevel` view — still no hyprland protocol, deliberately), zero
  `create_params`.
- The fallback matrix the probe did *not* run: at least one genuinely
  dmabuf-allocating client (video player, Qt dmabuf path) sharing a session
  with the global advertised — expected `failed` → shm fallback, proven on
  the wire rather than assumed. `foot` (shm) and `grim` already proven
  unaffected; they are the floor, not the matrix.
- The probe VM had `/dev/dri/card0`, so the `renderD128 → 0` fallback ladder
  never fired — yet `main_device=0` (no DRM node) is plausibly *the*
  production shape on GPU-less container targets (webtop). Exercise the
  no-node path explicitly, plus a `--nested` and a `--tty` re-drive of the
  acceptance above, before calling the advertisement done.
- `README.md`'s protocol inventory gains one row: what is advertised, what
  `failed` means here, and the one-sentence honesty statement from the
  measurement record. No config knob, no CLI flag, no IPC surface.

## Explicitly not in scope

- `hyprland-toplevel-export-v1`: still a compositor-specific protocol, still
  needs its own measured justification (the fallback entry's standing rule).
- Per-window thumbnails beyond readiness: that is the fallback entry's
  second half, possibly shell-side-only work, and it stays there.
- Empty format tables or omitted `main_device`: the measurement record
  explains why (client `mmap`/`qFatal` behavior); do not "simplify" into
  either without re-driving.

## The probe diff (base `9d265c2`, uncommitted working tree, reverted)

Reproducibility record for the measurement — 122 insertions across
`crates/flexwm/src/compositor/screencopy.rs`, 3 deletions (two import lines
reshaped, `Screencopy::new` gaining the two probe fields):

```diff
 use smithay::backend::allocator::Fourcc;
 use smithay::backend::renderer::{Bind, ExportMem};
-use smithay::output::{Output, WeakOutput};
+use smithay::backend::allocator::dmabuf::Dmabuf;
+use smithay::backend::allocator::{Format, Fourcc, Modifier};
+use smithay::backend::renderer::{Bind, ExportMem};use smithay::output::{Output, WeakOutput};
 use smithay::reexports::wayland_server::DisplayHandle;
 use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
 use smithay::reexports::wayland_server::protocol::wl_shm;
 use smithay::utils::{Buffer as BufferCoords, Clock, IsAlive, Monotonic, Rectangle, Transform};
+use smithay::wayland::dmabuf::{
+    DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
+};
 use smithay::wayland::image_capture_source::{
     ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
     OutputCaptureSourceState,
```

plus a `probe_dmabuf: DmabufState` field and a `probe_dmabuf_global:
Option<DmabufGlobal>` handle (`#[allow(dead_code)]`, probe-scoped) on
`Screencopy`; construction in `new()` via a `probe_dmabuf_state(dh)` helper
returning both; a `probe_dmabuf_feedback()` builder reading
`/dev/dri/card0` (else `renderD128`, else `0`, each logged) for
`main_device` and mapping [`FORMATS`](../../../crates/flexwm/src/compositor/screencopy.rs)
(`Xrgb8888` → `Fourcc::Xrgb8888`, `Argb8888` → `Fourcc::Argb8888`, anything
else warns and takes `Argb8888` rather than miscode) × `Modifier::Linear`
through `DmabufFeedbackBuilder::new(...).build()` (build failure skips the
global rather than half-advertising); and `impl DmabufHandler for State`
whose `dmabuf_imported` warns `PROBE: client attempted dmabuf import;
answering failed` and calls `notifier.failed()`.

(The `ExportMem};use` missing newline is verbatim as measured — cosmetic,
zero behavior; `fmt` will want it split.)

Rough size: S.
