---
title: "Shell window thumbnails without a toplevel capture protocol (region crop + quickshell's dmabuf readiness gate)."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Shell window thumbnails without a toplevel capture protocol.

Filed from the phase-1 probe that closed
[`../resolved/screencopy-toplevel-capture-done.md`](../resolved/screencopy-toplevel-capture-done.md)
as unreachable-for-the-motivating-client: stock quickshell 0.3.1 routes a
`Toplevel` `ScreencopyView` capture source exclusively to
`hyprland-toplevel-export-v1` (version-exact source + binary inventory + live
wire evidence in that file), so `ext_foreign_toplevel_image_capture_source_manager_v1`
was never built. This is the fallback that entry names: per-window thumbnails
as a **region capture out of the output** — `wlr-screencopy-unstable-v1` has no
toplevel source either, so there is no other protocol to reach for.

## Measurement TAKEN 2026-09-17 — YES (probe, nothing shippable)

**Verdict: YES — advertising `zwp_linux_dmabuf_v1` flips quickshell 0.3.1's
`isReady` readiness flag on GPU-less flexwm, and the already-shipped ext
output-capture path then displays over shm.** The overview preview half of
this entry is unblocked pending the advertisement itself, which is filed as
[its own implementation item](linux-dmabuf-advertisement.md) — a small
honest `linux_dmabuf` advertisement, not capture work. Per-window thumbnails
(the second half below) stay open behind it. The probe scaffolding was
reverted before review; the tree carrying this record is docs-only.

**Honesty, in one sentence:** every datum in the tried advertisement is true
(a real DRM `dev_t`, the two pixel formats the shm pipeline actually serves,
the layout shm buffers really have), but its tranche structure implies a
dmabuf *import* capability flexwm does not have — an overstatement no client
exercised (zero import attempts across three live sessions; quickshell cannot
by construction, see below), and any future attempt fails cleanly with the
protocol's own `failed`, so the verdict is *flips honestly enough to ship
behind coordinator say-so*, not *flips only by lying*.

### What was tried (one step; no escalation needed)

Base `9d265c2` plus an uncommitted PROBE diff to
`crates/flexwm/src/compositor/screencopy.rs` (122 insertions, reverted;
full text in the [follow-up item](linux-dmabuf-advertisement.md)): Smithay's
real `DmabufState` + `DmabufHandler` at the pinned rev, global version 6 via
`create_global_with_default_feedback`, feedback built by
`DmabufFeedbackBuilder` with `main_device` = `/dev/dri/card0`'s real rdev
(`57856` = `makedev(226, 0)`, logged at startup) and formats = exactly the
two the capture path serves (`Xrgb8888`, `Argb8888`) × `LINEAR`.
`dmabuf_imported` logged `PROBE` and answered `failed`.

Step 1 was the most honest shape available, so no Step 2 exists: source
analysis of quickshell 0.3.1 (`src/wayland/buffer/dmabuf.cpp`,
`manager.cpp`, `src/wayland/screencopy/view.cpp`, tag `v0.3.1`, fetched raw
and read in full) shows `done()` on the default feedback fires
`feedbackDone()` → `mDmabufFormatsReady = true` unconditionally — no
tranche, format or device content required — while `mRenderFormatsReady`
sets itself on the GPU-less failure path (`initWindow` sets it even as
`initRenderFormats` fails, hence the familiar SHM-fallback warning), and
`createBuffer` then skips dmabuf creation entirely (`mRenderFormatsFailed`)
straight to shm. The tranche *content* is inert for this client; only the
`done` matters. A fully content-free feedback (no table at all) would be
more honest still, but Smithay always sends the table and quickshell
`mmap`s it unconditionally with `qFatal` on failure — so an *empty* table
(`mmap` of length 0, always `EINVAL`) would abort the client rather than
merely not flip it. That determination is code-derived, not run live
(deliberately: no reason to crash the test client to prove a `mmap`
precondition), and it is why the probe starts at one real format, not zero.

### How it was driven, and what came back

Client: the exact PR #58 build — quickshell 0.3.1
(`/nix/store/hnw9kk48z8jqp0pqha5gwnpyawpcxq34-quickshell-0.3.1`,
`.quickshell-wrapped` sha256
`784914e5…b78b4`, full hash in that file). Drive: the same QML shapes
(`/var/tmp/qs-toplevel-probe.sh`, reused as-is with binary + tag args — one
`Toplevel` view plus one `Quickshell.screens[0]` view, two `foot` windows,
20 s under `WAYLAND_DEBUG=1`). All live work `--headless` on the dev VM;
binaries copied out of the shared target dir before running
(`/var/tmp/flexwm-dmabuf-control` from clean `9d265c2`,
`/var/tmp/flexwm-dmabuf-probe` from `9d265c2` + probe diff, force-clean
built with real `Compiling flexwm` lines, 27–28 s each). Compositor logs at
`/tmp/flexwm-tlprobe-dmabuf-<tag>.log`, wire at
`/tmp/qs-tlprobe-wire-dmabuf-<tag>.log`.

Control (`dmabuf-control`): `linux_dmabuf` 0 wire lines; both views `Cannot
capture frame, as no recording context is ready.`; `hasContent` 0 lines; no
ext-capture bind of any kind. The PR #58 baseline reproduces on current
`main`.

Probe (`dmabuf-probe`, reproduced identically on rerun `dmabuf-probe2`):

```
{Default Queue} wl_registry#2.global(9, "zwp_linux_dmabuf_v1", 6)
{Default Queue}  -> wl_registry#2.bind(9, "zwp_linux_dmabuf_v1", 5, new id [unknown]#30)
{Default Queue}  -> zwp_linux_dmabuf_v1#30.get_default_feedback(new id zwp_linux_dmabuf_feedback_v1#32)
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.main_device(array[8])
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.format_table(fd 19, 32)
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.tranche_target_device(array[8])
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.tranche_flags(0)
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.tranche_formats(array[4])
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.tranche_done()
{Default Queue} zwp_linux_dmabuf_feedback_v1#32.done()
```

(`format_table(fd, 32)` = the 2 probed entries × 16 bytes; `tranche_flags(0)`
is Smithay stripping `Sampling` for the v5 bind; mesa's egl queues bound v4
and took the same feedback — Qt-side probing, inert.)

- `no recording context is ready` drops from 2 warnings to 1 — the surviving
  one is the `Toplevel` view (`shell.qml:22`, alongside its `non captureable
  object`: PR #58 stands, nothing about the toplevel half changed).
- `QS: outview hasContent=true sourceSize=QSize(1200, 800)` — the overview
  shape displays, at the full output size.
- Its capture ran the shipped protocol path end to end: both views' ext
  globals bound (`ext_output_image_capture_source_manager_v1`,
  `ext_image_copy_capture_manager_v1`), `create_source` →
  `create_session` → constraints (`buffer_size(1200, 800)`,
  `shm_format(1)` then `(0)`) → two `create_frame`/`capture`s, both
  `ready`, into `wl_shm_pool.create_buffer(…, 1200, 800, 4864, 1)` —
  **wl_shm, format 1 (`Xrgb8888`, the opaque-first ordering PR #52 chose),
  64 bytes of row padding**, i.e. displayed over shm exactly as predicted.
- `create_params`/`create_immed`: 0 wire lines across all three sessions;
  `PROBE: client attempted dmabuf import`: 0 compositor lines (startup
  `PROBE` lines only). Nothing attempted an import flexwm cannot honor.
- Collateral: both `foot` windows mapped and listed normally (ordinary shm
  clients unaffected by the new global), and `grim` against the probe
  binary still captures (`800x600 8-bit/color RGB`, i.e. the shm/Xrgb path
  untouched).

### What was NOT run (stated, not implied)

- `main_device` alternatives (`renderD128`, `0`) and richer format tables:
  Step 1 flipped on first contact, so the ladder collapsed — recorded here
  so a future regression has somewhere to escalate.
- Other dmabuf-allocating clients (video players, Qt dmabuf paths): only
  `foot` (shm) shared the probe sessions. A client that binds the new
  global and tries to allocate gets `failed` and is expected to fall back
  to shm — that fallback matrix is the follow-up item's acceptance work,
  not this ticket's.
- `--nested` / `--tty`: probe ran `--headless` only. The advertisement
  code touches no backend; `--tty` additionally has a real scanout device,
  which only makes `main_device` *more* truthful there.

## One measurement to take first (it gates everything, including the overview)

The brief as filed — kept verbatim below so the answer above can be checked
against the question asked. Status: ANSWERED YES, 2026-09-17; the remaining
work is the [advertisement implementation](linux-dmabuf-advertisement.md)
and the thumbnail half underneath it.

The same probe turned up a second, wider gate: quickshell 0.3.1 never
instantiates *any* capture manager on flexwm — not even for the output path
PR #52 shipped — because `WlBufferManager::isReady()` requires
`mDmabufFormatsReady`, which is set only by real `zwp_linux_dmabuf_v1`
feedback events, which a GPU-less compositor truthfully never sends (full
mechanism in the resolved file). So today **every** quickshell
`ScreencopyView` is blank here: thumbnails for protocol reasons, *and* the
workspace-overview preview for buffer reasons despite its protocol working
(`grim` proves it).

Measure before anything else whether a minimal `zwp_linux_dmabuf_v1`
advertisement can flip that readiness flag on this compositor — *and*
whether such an advertisement can be truthful at all given flexwm has no
render node to describe (if readiness genuinely requires importing dmabufs
the compositor cannot produce, "minimal" is not honest and the answer is
no) — and whether the ext output-capture path then actually displays
over shm (quickshell falls back to shm buffer creation once ready, so in
principle nothing needs a real render node past the flag). If yes, the
overview preview lights up with no further protocol work, and the thumbnail
half below becomes a live question rather than a blank widget. If no — if
readiness genuinely requires importing dmabufs the compositor cannot produce
— then shell thumbnails on flexwm need the shells to change (a shm-only
readiness path upstream), and any compositor-side capture work for them is
moot until that lands. Either answer belongs in the resolved record, the same
way the toplevel probe's NO closed its ticket without a build.

## Then, for the thumbnails themselves

With readiness established, a per-window thumbnail is a client-side crop of
the output capture to the window's rect — the compositor already serves the
pixels (`grim` today), and the window geometry already reaches the shell
(`wlr-foreign-toplevel-management` gives position-relevant state; `flexwm msg
windows` gives exact rects for an agent). What, if anything, flexwm must do
beyond the pixels is the open part: possibly nothing (shell-side crop), which
would make this entry close the same way the toplevel one did — by
measurement, not code.

Explicitly out of scope here: implementing `hyprland-toplevel-export-v1` so
quickshell's existing `Toplevel` path lights up. It is a compositor-specific
protocol, not a standard, so `CLAUDE.md`'s rule points away from it; and it
sits behind the same readiness gate, so it cannot precede the measurement
above. If that measurement lands yes and a later probe shows the Hyprland
path is what DMS/Noctalia thumbnails actually need, file it as its own item
with that justification — do not smuggle it into this one.

Rough size: S for the measurement; unknown for whatever follows it.
