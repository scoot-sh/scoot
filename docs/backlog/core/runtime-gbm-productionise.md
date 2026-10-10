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

## What to do

1. Take the M2 scanout proof the spike could not: ship the spike build by
   tar over ssh, run `--tty --renderer gles` on a free VT (check for other
   agents' `scoot`/`seatd`/`openvt` first), confirm the tier engages and
   scans out, record SHA + raw output in `Asahi.md`.
2. Decide the auto-detect rule and implement it in `render::resolve` /
   `tty::init`: default to the GPU tier only when a render-capable device
   plus loadable EGL/GBM actually probe (one-line log on fallback);
   explicit `--renderer` / config always wins; a forced-GPU request
   without libgbm fails loudly, never silently.
3. Collapse the feature: `runtime-gbm` becomes the default (or merges into
   `gpu-scanout`), `flake.nix` `scoot`/`scoot-gpu` collapse to one package
   (the four dependency artifacts collapse with them), CI's `ldd` gate
   asserts the single binary links no libgbm, and
   `install.md#which-build-do-i-need` loses the CPU/GPU axis (Xwayland Axis
   stays until its own ticket lands).
4. Measure at merge time and record here: binary size delta, startup time
   delta, and that no heap allocation landed on a per-frame path (the stub
   runs at startup/resize/import only).

## Not in this ticket

The `xwayland`-always follow-up (`./core/xwayland-always.md`): same shape,
separate measurement. Auto-selecting `--renderer gles` on boxes that
already pass `--renderer gles` explicitly changes nothing there.
