---
title: "Productionise runtime-loaded libgbm: one default build for GPU and GPU-free boxes"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-13"
---

# Productionise runtime-loaded libgbm: one default build for GPU and GPU-free boxes

# Productionise runtime-loaded libgbm: one default build for GPU and GPU-free boxes

Filed 2026-10-10 (spike `runtime-gbm`). Serves **daily-drive**: today a
user must pick the right binary (`scoot` vs `scoot-gpu`) before knowing
whether the box has a usable GPU stack; one build removes the chooser's
sharp edge and halves the compositor packaging matrix.

## The gap

`gpu-scanout` is a link-time libgbm dependency (`gbm-sys`'s
`#[link(name = "gbm")]`), so a scanout-capable binary will not start where
libgbm is absent, and the default build cannot contain the GPU tier at all
(`crates/scoot/Cargo.toml`, `flake.nix` `scoot` vs `scoot-gpu`,
`site/src/content/docs/start/install.md#which-build-do-i-need`).

The spike (feature `runtime-gbm`, `crates/scoot/gbm-stub/gbm_stub.c`)
proves the mechanism: a static ABI stub (36 forwarded symbols, the complete
FFI surface of the pinned `gbm` 0.18.0) satisfies `-lgbm` with no DT_NEEDED
entry -- `readelf -d`/`ldd` clean both with and without a system libgbm --
and fails closed (`ENOSYS`) where none is installed, which the existing
fallbacks already handle (`tty::try_scanout` warns and keeps dumb buffers;
`nested::gpu` keeps read-back). The spike build stays behind its
off-by-default feature; the default build is untouched.

## Goal (user, 2026-10-10): one binary, easy to swap

The end state is ONE binary in which the CPU and GPU renderers are EQUAL
tiers: `auto` chooses the best one for the hardware at hand, and the user
can always override it. GPU-free operation stays a hard requirement
(webtop/no-GPU boxes must keep working exactly as today), not something a
GPU path supersedes. So auto-select must be hard-aware rather than
GPU-eager: on software GL (llvmpipe/softpipe, software EGL) and
known-slow virtual GPUs (virtio-gpu, vmwgfx, QXL) the CPU path may be
faster, and there auto must pick CPU -- while an explicit choice always
wins either way. Swapping must be easy: a flag, a config knob, an env
var -- ideally also at runtime without restarting clients. This PR
(#547) stays the off-by-default spike; productionisation -- default-on,
auto policy, swap UX -- is this ticket.

## What to do

1. Take the M2 scanout proof the spike could not: ship the spike build by
   tar over ssh, run `--tty --renderer gpu` on a free VT (check for other
   agents' `scoot`/`seatd`/`openvt` first), confirm the tier engages and
   scans out, record SHA + raw output in `Asahi.md`.
2. Land the auto policy per `renderer-auto-policy` (pure
   `render/policy.rs`: KMS-driver + GBM/EGL + `GL_RENDERER` checks, one
   INFO line, `auto` never fails startup; an explicit request always wins
   over `auto`; a forced-`gpu` request without the stack fails loudly,
   never silently). This ticket owns the collapse and the default flip
   around that policy, not the table itself; do not fix the table before
   the exploration measurement in item 6.
3. Collapse the feature: `runtime-gbm` becomes the default (or merges into
   `gpu-scanout`), `flake.nix` `scoot`/`scoot-gpu` collapse to one package
   (the four dependency artifacts collapse with them), CI's `ldd` gate
   asserts the single binary links no libgbm, and
   `install.md#which-build-do-i-need` loses the CPU/GPU axis (Xwayland Axis
   stays until its own ticket lands).
4. Measure at merge time and record here: binary size delta, startup time
   delta, and the per-frame stub cost at the export path (see "Per-frame
   cost" below).
5. Harden the link (review Finding 4, PR #547): the mechanism needs
   build.rs's `static=gbm` to resolve the `gbm_*` refs *before* gbm-sys's
   plain `-lgbm` is processed under `--as-needed`, so the shared object is
   never marked NEEDED -- nothing in cargo/rustc contracts that order. A
   flipped order fails loudly (a `DT_NEEDED libgbm` the `ldd` gate
   catches; the archive simply goes unpulled), which is why the spike could
   merge -- but before collapse, verify `readelf -d` clean on debug AND
   release (fat LTO, `Cargo.toml` workspace profile: `lto = "fat"`,
   `codegen-units = 1`) AND nix/crane AND an lld-or-mold link, and record
   the ordering assumption beside the `ldd` gate so the debugger of a
   future gate failure knows why ("`static=gbm` must precede plain `-lgbm`
   under `--as-needed`"). Note for packagers: building with the feature
   needs a C toolchain (`cc` build-dep); without it the path is inert.
   CI must build AND test with the feature on, plus keep the `ldd` gate --
   otherwise the first `gbm` bump breaks the stub with nobody noticing
   (the "fails the link loudly" backstop only works if some builder
   builds it).
6. Exploration (before fixing the auto-policy table): measure CPU vs
   GPU frame time and CPU use in the dev VM (`vm/README.md`) and on the
   M2, and record both here. The VM's virtio-gpu has no `IN_FORMATS` and
   no `ADDFB2_MODIFIERS` (see `tty/scanout.rs` header), so the comparison
   may favor the CPU tier there -- that is the expected hardware-aware
   outcome, not a failure.

## CPU-only minimal build (user, 2026-10-10; stage C gate)

A minimal CPU-only build must stay possible via Cargo features
(`cargo build --no-default-features`): no GBM code, no C stub, no C
compiler need, no libgbm anywhere, and XWayland excludable too. In such
a build `--renderer gpu` fails at startup with a clear message and
`auto` resolves to cpu with reason `no-scanout-tier` (the
`renderer-auto-policy` table already rows this; the build must keep that
row reachable). The mechanism half already holds: `build.rs` compiles
the stub only under `CARGO_FEATURE_RUNTIME_GBM` (so without the feature
no C compiler runs), and `default = []` carries neither `gpu-scanout`
nor `xwayland` — stage C (the default flip / feature collapse in item 3)
must leave this path intact, not absorb it. Concretely: (a) the collapse
keeps the scanout tier behind a feature the minimal build disables;
(b) a CI job builds `--no-default-features`, asserts no libgbm in
`readelf -d`, no stub symbols via `nm`, and that the binary starts;
(c) a documented/packaged minimal build exists (e.g. a nix
`scoot-minimal` or a documented command) so the path is a product, not a
flag combination. See `xwayland-always.md` for the XWayland half (the
feature stays disableable there too).

## Per-frame cost (measured for the spike, PR #547 fix round)

Review Finding 1 (PR #547): the stub comment's "no per-frame hot path"
claim was false. At the pinned Smithay rev every presented frame on the
scanout tier calls `GbmBufferedSurface::next_buffer` -> `slot.export()`
-> `Exporter::export`: `plane_count()` plus per plane `fd_for_plane()`,
`offset()`, `stride_for_plane()` -- ~1 + 3xplanes stub calls per frame,
each paying a `dlsym`. Microbenchmark (dlopen real libgbm.so.1, `dlsym` a
real symbol per call, 2M iterations best-of-3, linux/aarch64 container):
~20 ns per `dlsym`-then-call vs ~1 ns cached-pointer call, i.e. ~0.08 us
per frame at 1 plane / ~0.14 us at 2 planes -- ~0.001% of a 16.7 ms
(60 Hz) / 6.9 ms (144 Hz) budget. Negligible: no pointer caching, by
measurement, not by assertion. Re-measure if the export path changes.
Raw numbers live in the fix-round report (`report-fix-547.md` in the
orchestrator scratchpad) and the PR discussion.

## Easy swapping: design for this ticket

- **Flag/config/env.** `--renderer cpu|gpu|auto` and `[renderer]
  backend` already exist in spirit (`--renderer pixman|gles` plus
  `[renderer] backend` today; the user-facing names change to `cpu|gpu`
  in the renderer-auto-policy PR, breaking, no aliases -- this ticket
  uses the new names throughout). Precedence (per `renderer-auto-policy`,
  flag > env > config > default): an explicit `--renderer` wins,
  `SCOOT_RENDERER` (following the existing `SCOOT_SOCKET` /
  `SCOOT_TEST_RENDERER` precedent) beats the file, the file beats the
  built-in default; unknown names are refused, never defaulted -- keep
  that. `auto` means "run the auto policy below" and becomes the default
  only when this ticket lands; until then the default stays CPU.
- **Runtime switch.** Research (not yet proven): switching tiers over
  IPC / `scoot msg` without restarting clients. Cost to scope: re-create
  swapchains and presenters per output while clients stay mapped -- the
  same teardown-first discipline as the seat reconnect that merged as
  #548 (drop the old device/session objects before building the new
  ones, never both half-alive). If the scope proves larger than a
  follow-up, land flag/config/env + auto first and keep the runtime
  switch as its own ticket.
- **Auto policy.** The two tiers are equal: `auto` picks whichever fits
  the probed hardware -- GPU where a real accelerated stack loads, CPU
  where software-only or a known-slow virtual GPU. An explicit choice
  always wins; a forced-`gpu` request without the stack fails loudly.
  Detectable signals:

  | Signal | Source | Means |
  |---|---|---|
  | DRM driver name | `drmGetVersion` on the render node | `virtio_gpu`, `vmwgfx`, `qxl` -> known-slow virtual GPU -> CPU |
  | EGL renderer string | `eglQueryString(EGL_RENDERER)` | `llvmpipe` / `softpipe` / `Software` -> software GL -> CPU |
  | GBM probe | stub `scoot_gbm_real_loaded` | 0 -> no libgbm -> CPU, no questions |
  | EGL probe | existing `lib_loadable` | un-loadable -> CPU |
  | Dumb-buffer fallback | existing `try_scanout` warn path | any refusal above -> CPU + one-line log |

  Benchmark both tiers in the dev VM before fixing this table (item 6):
  if the CPU tier wins on virtio-gpu, the table ships saying so.

## Not in this ticket

The `xwayland`-always follow-up (`./core/xwayland-always.md`): same shape,
separate measurement. Auto-selecting `--renderer gpu` on boxes that
already pass `--renderer gpu` explicitly changes nothing there.

## Measurements (stream G, 2026-10-14, single binary)

Collapse decision: `runtime-gbm` merged INTO `gpu-scanout` (the spike
feature is gone), `default = ["gpu-scanout"]`. One feature per tier,
not two features for the same Smithay code: the old link-time behavior
(`DT_NEEDED libgbm`) is gone on purpose, and the stub is proven below
on every link flavor that matters.

**Item 1, M2 scanout proof** (source `255c7cc9f`, spike
`gpu-scanout,runtime-gbm` release build, own dir/target on the M2):
`readelf -d` NEEDED clean (no libgbm/EGL/drm); on VT2
`GL Renderer: "Apple M2 (G14G B0)"`,
`renderer chosen requested=gpu source=flag tier=gpu reason=requested
driver=apple`, `scanout="gpu"` on eDP-1 2560x1600, `scoot is up …
renderer=gpu`; `outputs` live, screenshot 2560x1600 captured
(`~/fx/onebuild-g-proof-gpu.png` on the box). Bare-env first run fell
back as designed (`no libEGL.so.1` → `tier=cpu
reason=gpu-build-failed` + dumb). `--renderer auto` (the default):
`tier=gpu reason=hardware-renderer`. Seat released, tty1 untouched.

**Item 5, link matrix** (`readelf -d` NEEDED, no libgbm): release fat
LTO (M2) clean; debug (dev VM) clean with the `ldd` soname gate clean;
lld link (`Linker: LLD 21.1.8` in `.comment`) clean — the
`static=gbm`-before-`-lgbm` ordering holds under lld's `--as-needed`
too. nix/crane and mold: CI-only (dev VM disk is the constraint).
Ordering assumption recorded beside the `ldd` gate (`ci.yml`), in
`build.rs` and in `crates/scoot/Cargo.toml`. Two gate false-positives
found live and fixed: `nm | grep -i gbm_` hits hashbrown's `TagBM_E…`
(gate names canaries now), and bare `readelf -d | grep libgbm` hits the
dev shell's RUNPATH mesa-libgbm path (gates scope to NEEDED) — that
RUNPATH is what the stub's `dlopen` resolves through on NixOS.

**Item 4, sizes** (release fat LTO aarch64, this change): single
7,630,928 (7.3 MB); minimal 7,037,240 (6.7 MB); `+xwayland` 8,749,552
(8.3 MB). Collapse delta +593,688 (+8.4%) over minimal for tier+stub.
Startup (launch → `scoot is up`, 5 alternating runs): M2 release
`--tty` cpu mean 0.6391 / auto mean ≈0.641 (delta +2 ms); dev VM debug
`--tty` cpu mean 0.5174 / auto mean 0.5190 (delta +1–2 ms, auto
`no-egl` in the bare-ssh env). No startup regression either way.
Per-frame stub cost: not re-measured — Smithay rev identical since the
spike, stub byte-identical, `gbm` still 0.18.0 — standing ~0.08
µs/frame (1 plane) / ~0.14 µs/frame (2 planes).

**Item 6, CPU vs GPU** (`scripts/tty-tier-bench.sh`, one binary,
`--renderer cpu` vs `--renderer gpu`, 4 rounds, medians, jiffies =
compositor utime+stime):
- Dev VM (virtio-gpu, llvmpipe GLES in `nix develop`; bare-ssh attempt
  void on missing libEGL): move medians cpu 58 vs gpu 65 jiffies
  (~+12%, near noise); width medians cpu 116 vs gpu 435.5 (~3.8x
  dearer); idle 0 both; rss ~64 vs ~160 MB. CPU wins decisively where
  it matters — the expected outcome; `virtual-gpu → cpu` stays.
- M2 (eDP-1, private seatd, release single; two system-seat attempts
  void on a held seat): move medians cpu 122 vs gpu 35 (~3.5x less
  CPU); width medians cpu 617.5 vs gpu 171.5 (~3.6x less CPU); idle 9
  vs 2 jiffies/10 s; rss ~120 vs ~140 MB. Reproduces the standing
  record on the single binary; `hardware-renderer → gpu` validated.

**Stage C gate (minimal product):** `--no-default-features` builds
with `CC` pointed at a failing script (no C compiler need);
`readelf -d` NEEDED clean; canary symbols
(`gbm_create_device`/`scoot_gbm_real_loaded`) absent; binary starts
(headless smoke green, `tier=cpu`). `--renderer gpu` on `--tty`
warns `this build has no gpu scanout tier…` and keeps cpu with reason
`no-scanout-tier`; `auto` resolves cpu with `no-scanout-tier` (an
`open_device` early-out added for this: without it the device-time
policy overrode the row with a misleading `gpu-build-failed`). Nix
`packages.scoot-minimal` + CI minimal job own the product.

## Resolution (2026-10-13)

Landed in #588 (`feat(scoot)!:`, breaking): the stub merged into
`gpu-scanout` (default-on, `runtime-gbm` feature gone), flake collapsed
to `scoot` + `scoot-xwayland` + `scoot-minimal`, CI owns the minimal
product gate, docs rewritten to one build. Evidence: M2 scanout proof
with screenshot, both tier benches, and the link matrix live in the
stream report (orchestrator scratchpad `onebuild-g-report.md`); suite
counts (nextest 2471/2298, cargo test both, both smokes) in the PR body.
The XWayland axis stays open in `../core/xwayland-always.md`.
