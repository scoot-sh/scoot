---
title: "Productionise runtime-loaded libgbm: one default build for GPU and GPU-free boxes"
status: "open"
area: "core"
priority: "medium"
blocked: "spike PR review; Asahi M2 scanout proof"
---

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
2. Decide the auto-detect rule and implement it in `render::resolve` /
   `tty::init`: `auto` picks the tier that fits the probed hardware (a
   render-capable device plus loadable EGL/GBM -> GPU, otherwise CPU),
   with a one-line log saying which tier was chosen and why; an explicit
   `--renderer` / config always wins over `auto`; a forced-`gpu` request
   without the stack fails loudly, never silently. See "Auto policy"
   below for the hardware-aware table; do not fix the table before the
   exploration measurement in item 6.
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

- **Flag/config/env.** `--renderer cpu|gpu|auto` and `[renderer] mode`
  already exist in spirit (`--renderer pixman|gles` plus `[renderer]
  backend` today; the user-facing names change to `cpu|gpu` in the
  renderer-auto-policy PR, breaking, no aliases -- this ticket uses the
  new names throughout). Precedence stays: flag wins over file, env
  overrides both (e.g. `SCOOT_RENDERER`, following the existing
  `SCOOT_SOCKET` / `SCOOT_TEST_RENDERER` precedent); unknown names are
  refused, never defaulted -- keep that. `auto` means "run the auto
  policy below" and becomes the default only when this ticket lands;
  until then the default stays CPU.
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
