---
title: "Nix package can reach neither GPU tier (no EGL in RUNPATH, no gpu-scanout build)"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Nix package can reach neither GPU tier

Filed as gh issue #177 (read it — live Asahi M2 evidence, exact panic,
`readelf`/`ldd` proofs, and a confirmed-working `LD_LIBRARY_PATH`
workaround proving the renderer itself is fine on the hardware; this
entry tracks it).

Two independent causes, fixable separately:

1. **`--renderer gles` panics: no EGL at runtime.** Smithay reaches
   libEGL through `dlopen`, not link-time `DT_NEEDED`, so the cc
   wrapper's RUNPATH logic (correct for `-lfoo`) puts nothing in the
   RUNPATH (`readelf -d … | grep -i runpath` empty). Failure is a panic
   in Smithay's ffi (`Failed to load LibEGL`), not the loud startup
   error stage 2 designed — that logic never runs because the panic
   happens inside `dlopen` before device enumeration. Fixes: `makeWrapper`
   setting `LD_LIBRARY_PATH`, or `postFixup` adding the libglvnd path to
   RUNPATH. Decide explicitly whether Mesa (vendor ICD) comes from the
   derivation or the system. Consider a pre-flight check so a box with no
   EGL reports the designed startup error instead of a backtrace.
2. **No `gpu-scanout` build.** The feature is correctly off by default
   (link-time libgbm, GPU-free operation is fixed), but the flake passes
   no `--features` anywhere, so Test 4's tier B is unreachable from the
   package (Asahi.md says it needs a `--features gpu-scanout` build).
   Proposed: `packages.scoot-gpu` via `scoot.overrideAttrs` with
   `buildFeatures = [ "gpu-scanout" ]` — also gives CI's libgbm positive
   control a packaged counterpart (pairs with #173).

Why now: the issue's machine IS the Test 4 machine (split render/display:
AGX `card1` render node, `apple,dcp` `card2` connectors — never
exercised). Reaching the test by leaving the packaging behind makes the
result unreproducible.

Scope: flake packaging only (plus the possible pre-flight check, which is
small compositor code — keep it minimal or file it separately if it
grows). No renderer changes. Hardware proof stays on the Asahi machine;
what can be proven here (packaged `gles` reaching llvmpipe EGL on the dev
VM, `scoot-gpu` linking libgbm) must be proven here.
Also in the issue (do not lose): `docs/roadmap/README.md` row 6 says
planned while `06-gpu-pipeline.md` says in-progress — one is stale (also
filed as #178.6; fix once, reference both).
