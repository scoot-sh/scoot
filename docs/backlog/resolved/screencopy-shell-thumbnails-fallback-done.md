---
title: "Shell window thumbnails without a toplevel capture protocol — CLOSED: overview verified on shipped main, thumbnails need upstream (measured, no build)."
status: "resolved"
area: "resolved"
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

## CLOSED 2026-09-17 — overview YES on shipped `main`, thumbnails (c) NEEDS-UPSTREAM (docs-only, no build)

Step 1 re-drove the PR #58 quickshell shapes against current `main`
(`42c17f0`, which carries the shipped PR #60 advertisement — no probe
scaffolding): the overview-shape view lights up with real pixels. Step 2
proved the client-side crop mechanism live on that same tree and read the
shell-side sources at their current upstream revisions: the compositor
serves everything a crop needs, and both shells need to change to use it —
DMS hard-requires a `Toplevel` source, and current Noctalia has no
per-window live-thumbnail view at all. No compositor work follows (not
outcome (b)), and `hyprland-toplevel-export-v1` stays unimplemented per
the standing rule. No README user-facing change: behavior is exactly what
PR #60 shipped; this entry only measures it.

### Step-1 re-drive evidence (all live `--headless` on the dev VM)

- Base: clean `42c17f0` through the 9p mount, force-clean built
  (`cargo clean -p flexwm && cargo build -p flexwm`: real `Compiling
  flexwm` line, `Finished in 28.99s`), copied out before driving:
  `/var/tmp/flexwm-ship-redrive`, sha256 `0542b95d…6cc52fa36e`.
- Client: the exact PR #58 build — quickshell 0.3.1
  (`/nix/store/hnw9kk48z8jqp0pqha5gwnpyawpcxq34-quickshell-0.3.1`,
  `.quickshell-wrapped` sha256 `784914e5…b78b4`); windows `foot`
  1.28.0. Drive: `/var/tmp/qs-toplevel-probe.sh` reused as-is
  (`/var/tmp/qs-toplevel-probe.sh /var/tmp/flexwm-ship-redrive
  ship-redrive`, 20 s, `WAYLAND_DEBUG=1`). Compositor log
  `/tmp/flexwm-tlprobe-ship-redrive.log`, wire
  `/tmp/qs-tlprobe-wire-ship-redrive.log`.
- quickshell binds `zwp_linux_dmabuf_v1` v5 on the Default Queue and
  receives the full default feedback through `done()`
  (`format_table(fd, 32)` = the 2 shipped entries × 16 bytes); mesa
  binds v4 on its own queue. Compositor startup logs the real scanout
  device (`dmabuf feedback main device device=57856
  source="/dev/dri/card0"`).
- `QS: outview hasContent=true sourceSize=QSize(1200, 800)` — the
  overview shape displays at full output size, via the shipped ext path
  (`create_source` → `create_session` → `buffer_size(1200, 800)` →
  `shm_format` → `create_frame`/`ready`) into
  `wl_shm_pool.create_buffer(…, 1200, 800, 4864, 1)` (wl_shm,
  `Xrgb8888`, 64 bytes row padding).
- Still standing: one surviving `no recording context is ready` (the
  `Toplevel` view, `shell.qml:22`, plus its `non captureable object`),
  zero `create_params`/`create_immed` wire lines — PR #58's toplevel
  verdict unchanged.
- Pixel proof, not just `hasContent`: IPC screenshots mid-session
  (`/tmp/flexwm-shot-ship.png` vs control `/tmp/flexwm-shot-control.png`
  from the still-present pre-advertisement binary
  `/var/tmp/flexwm-dmabuf-control`; both fetched locally and compared
  pixel by pixel). Control
  outview/winview rects are flat `#303030` (the blank panel); ship
  outview rect holds 16 distinct colors (13 substantive terminal grays
  plus white and two singletons) while
  the winview rect stays flat `#303030`. The overview preview shows
  real captured pixels on shipped `main`.

### Step-2 verdict: (c) NEEDS-UPSTREAM, with the recipe proven live

What a shell must do (all of it already servable, measured below): bind
a screen-source `ScreencopyView` (`Quickshell.screens[0]`, the shipped
ext output path), place it full-output-size inside a clipped container
positioned at the window's rect with the view offset by `(-x, -y)`, and
read the rect from `wlr-foreign-toplevel-management` (live —
`ToplevelManager` populated 2/2 in every session here) or
`flexwm msg windows` (exact `rect` per window, verified live). No
`ScreencopyView` crop property exists to use instead (version-exact
`view.hpp` at quickshell tag `v0.3.1`: `captureSource`, `paintCursor`,
`live`, `hasContent`, `sourceSize`, `constraintSize`, `captureFrame()`
— no region/sourceRect), so the clip+offset container *is* the
client-side crop, and it needs no compositor help.

That recipe was driven live, not reasoned (`/var/tmp/qs-crop-probe.sh`,
kept on the dev VM; fullscreen transparent `PanelWindow`,
`ExclusionMode.Ignore`, clip `Item` at the `msg windows` rect of
`alpha` (12, 12, 582×776) holding a 1200×800 screen-source view;
`foot -o cursor.blink=no` for pixel-stable windows; wire
`/tmp/qs-crop-wire-crop2.log` runs the shipped path with zero warnings):

- `QS: cropview hasContent=true sourceSize=QSize(1200, 800)`.
- Stale-frame proof: after the view's captures, `msg action
  focus-window-id 1` + `msg type "HELLO-THUMBNAIL"` + `msg key Return`
  changed the live window; three IPC screenshots compared pixel by
  pixel over the 451,632-px rect (`/tmp/flexwm-crop-crop2-preref.png`
  clean pre-typing, `/tmp/flexwm-crop-crop2.png` in-session,
  `/tmp/flexwm-crop-crop2-postref.png` clean post-typing):
  in-session ≡ pre-ref with **0 px differ** (the view shows its capture
  faithfully), while in-session vs post-ref differs by exactly the 973
  typed-glyph px in rows 12–51 — the same 973 px pre-ref vs post-ref
  differs by. The view displays real captured window pixels, stale
  across the later content change: the crop mechanism works end to
  end on flexwm as-shipped.
- Method note: crop-probe v1 used an opaque red backing rect to locate
  the crop and photographed its own backing (solid-red region,
  correctly diagnosed from the pixel grid, not mistaken for a
  compositor finding); v2 is transparent and the staleness comparison
  above is what carries the verdict.

Why that still closes as needs-upstream rather than shell-config:

- DMS (quickshell-based, read at master HEAD 2026-09-17,
  `AvengeMedia/DankMaterialShell@641d65cb`, `captureSource` line re-verified
  at `:90` on that SHA):
  `quickshell/Modals/DankLauncherV2/TileItem.qml:90` binds
  `captureSource: root.waylandToplevel`, where `waylandToplevel`
  (lines 28–34) is `pluginInstance.getToplevelById(toplevelId)` and
  `hasScreencopy` (line 36) is `waylandToplevel !== null`. There is no
  screen-source path and no config knob: on flexwm the wlr
  `ToplevelManager` populates (so icons hide), the `Toplevel` source
  hits quickshell's hyprland-only route ("non captureable object"),
  and tiles render blank. DMS must adopt the screen-source + clip crop
  above (or equivalent) — a shell change.
- Noctalia (current upstream is the native C++ `noctalia-dev/noctalia`
  monorepo at `8c52cb71`; no QML remains): the taskbar is icon-based
  (`taskbar_widget`), `ThumbnailService` decodes image files (not live
  windows), and the capture stack (`src/capture/screencopy_capture.*`)
  speaks only `zwp_screencopy_manager_v1` (`capture_output(_region)`)
  for screenshots — a global flexwm deliberately does not advertise
  (ext-only, per the measured PR #52 call). There is no per-window
  live-thumbnail view to feed; its screen-source overview/lockscreen
  path is servable over the shipped ext protocol.
- quickshell 0.3.1 itself (both shells' toolkit where QML applies;
  `outfoxxed/quickshell` tag `v0.3.1`):
  `ScreencopyManager::createContext` routes a `Toplevel` source
  exclusively to `hyprland-toplevel-export-v1` (PR #58 leg 2 stands,
  re-confirmed against the live `non captureable object` above), which
  stays out of scope without its own measured justification.

No (b) follow-up is filed: nothing in the recipe wants compositor work
— pixels (ext output capture), readiness (PR #60 advertisement), and
geometry (`wlr-foreign-toplevel-management` live plus `msg windows`
exact rects) are all shipped and all re-verified on the closing tree.

### What was NOT run (stated, not implied)

- The full DMS / Noctalia shells were never driven live (deps beyond
  this ticket); the shell-side verdict rests on their shipped sources
  at stated revisions plus the quickshell mechanism probes above.
- `--nested` / `--tty` re-drives of the crop: the crop is a client-side
  arrangement of the already-shipped output path (proven on all four
  backends by PR #60); backend-specific re-proof would re-execute
  byte-identical dispatch.
- No benchmark: probe, not a hot path.
- Incidental observation, not filed: `flexwm msg windows` with its
  stdout reader gone (missing `python3` on the dev VM broke the pipe)
  panics in the *client* (`failed printing to stdout: Broken pipe`) —
  cosmetic, compositor unaffected, noted for triage.
