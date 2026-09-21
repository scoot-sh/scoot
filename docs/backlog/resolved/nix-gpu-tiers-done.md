---
title: "Nix package can reach neither GPU tier (no EGL in RUNPATH, no gpu-scanout build) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Nix package reaches both GPU tiers — RESOLVED 2026-09-21

Filed as gh issue #177 (live Asahi M2 evidence, exact panic,
`readelf`/`ldd` proofs, and a confirmed-working `LD_LIBRARY_PATH`
workaround proving the renderer itself is fine on the hardware).
Closed by the PR that moved this file; `Fixes #177` in its body.

Two independent causes, fixed separately (no renderer changes — the
renderer was proven fine on the M2 by the issue's workaround run):

1. **Packaged `--renderer gles` panicked: no EGL at runtime.** Smithay
   reaches libEGL through `dlopen`, not link-time `DT_NEEDED`.
2. **No `gpu-scanout` build existed in the flake.**

## Decision 1 — EGL runtime: force the link, niri-style (not a wrapper)

`packages.scoot` sets derivation-only `env.RUSTFLAGS` with
`-Wl,--push-state,--no-as-needed -lEGL -Wl,--pop-state`, the exact trick
nixpkgs' own niri package uses (`pkgs/by-name/ni/niri/package.nix`:
"Force linking with libEGL ... so they can be discovered by `dlopen()`").
That lands `libEGL.so.1` in `DT_NEEDED`, which the loader resolves through
the RUNPATH the cc wrapper already builds from `buildInputs` -- so the
`dlopen` finds the already-loaded handle. No wrapper script, no
`LD_LIBRARY_PATH` leaking into every spawned client, and `readelf -d`
shows the `NEEDED` entry as the audit. A `postFixup`
`patchelf --add-rpath` would do the same job with a hand-computed store
path; this reuses the wrapper's own path computation instead.

Two deviations from the ticket, both proven by the build:

- The ticket proposed `makeWrapper`/`LD_LIBRARY_PATH` or `postFixup`
  RUNPATH. The niri precedent (same Smithay `dlopen`, same nixpkgs) is
  more idiomatic than either and keeps the binary self-describing; picked
  with that reasoning recorded in `flake.nix`.
- The flags are Linux-gated (`optionalString ... isLinux`): Apple's ld
  rejects `--push-state` (`ld: unknown option`, proven by a failed Darwin
  build), and there is no libEGL on Darwin anyway.

Deliberately derivation-only, never in-tree: CI's `ldd` gate asserts the
plain `cargo build` links no libEGL, and it still passes (verified below).

**Mesa ICDs come from the host OS, not the derivation** (decided
explicitly, as the ticket asked): the package ships libglvnd dispatch;
vendor drivers come from the system OpenGL setup (NixOS
`hardware.graphics`) -- the standard nixpkgs pattern. Bundling Mesa would
risk shadowing the host's drivers (notably Asahi's) with wrong ones.
`docs/nix.md` states this.

One correction to the issue's evidence, re-derived rather than relayed:
its `readelf -d … | grep -i runpath` printed empty, but the pre-fix binary
rebuilt here **does** carry a RUNPATH -- for the link-time deps only. The
precise mechanism is narrower: `NEEDED` has no libEGL and the RUNPATH has
no libglvnd entry, so only the `dlopen` fails. Recorded from the build
below, not the issue text.

## Decision 2 — pre-flight probe: built (it stayed tiny)

`compositor/render/gles.rs::lib_loadable` probes `libEGL.so.1` with
`libc::dlopen` (no new dependency -- `libc` is already direct) before
Smithay's first EGL touch, whose miss handler is
`.expect("Failed to load LibEGL")` (`ffi.rs:148` at the pinned rev, the
only `Library::new(...).expect(...)` in either backend, checked in
source). Both GLES tiers call it first: the offscreen tier maps the miss
to the designed startup error with the pixman fallback hint,
`ScanoutBackend::new` returns it to `try_scanout`, which falls back to
CPU/dumb with a warning. `catch_unwind` was not an alternative (release
`panic = "abort"`). ~50 lines plus two unit tests; the panic-backtrace
shape is fixed, not ticketed.

## Decision 3 — `packages.scoot-gpu`: shipped, with one correction

`scoot.overrideAttrs` with a new `pname`, inheriting everything
(`cargoBuildFlags`, `buildInputs` -- libgbm was already in
`vm/compositor-deps.nix` -- EGL forcing, stripping). **Correction: the
ticket proposed `buildFeatures = [ "gpu-scanout" ]`, which the build
proved a silent no-op** -- `buildFeatures` is consumed when
`buildRustPackage` is *called*; `overrideAttrs` can only change the
resulting derivation's own attrs, so it built an unfeatured binary
(proven by `ldd`: no libgbm). The working attr is `cargoBuildFeatures`,
what the cargo hook actually reads; the rebuild log shows
`--features=gpu-scanout`. Plus an `apps.scoot-gpu` entry
(`nix run .#scoot-gpu -- --tty -- ...`) for the Test 4 run.

## Decision 4 — roadmap row: fixed toward done-on-paper

Verified, not guessed: stage PRs #129 (seam), #130 (GLES pipeline),
#133 + #135 (split stage 3: presenter split, then scanout), #147
(stage 4) are all merged, and `ROADMAP.md` already calls item 6 "done
on paper — unverified on a real GPU". So `docs/roadmap/README.md` row 6
(`planned`) and `06-gpu-pipeline.md` (`in-progress`) were both stale in
the same direction; the row now reads done-on-paper with all five PRs,
the file's frontmatter reads `done`. This also closes #178.6.

## Evidence (recorded, not narrated)

All Nix/Linux commands ran on the dev VM (`ssh -p 2222 dev@localhost`,
aarch64 NixOS, tree at `/mnt/scoot`, branch `backlog/nix-gpu-tiers`);
Darwin commands on the Mac (arm64, `/Users/steveyackey/code/flexwm`).

BEFORE (packaged `scoot` from `main` at `270853c`,
`nix build "git+file:///mnt/scoot?rev=270853c#scoot"`, out-link
`/tmp/scoot-before`):

- `readelf -d /tmp/scoot-before/bin/scoot`: `NEEDED` has no libEGL;
  RUNPATH has no libglvnd entry.
- `/tmp/scoot-before/bin/scoot --headless --width 640 --height 480
  --renderer gles` → `thread 'main' panicked at
  .../smithay-0.7.0/src/backend/egl/ffi.rs:148:65: Failed to load LibEGL:
  DlOpen { desc: "libEGL.so.1: cannot open shared object file: No such
  file or directory" }`, exit 134.

AFTER (`nix build .#scoot` → `/tmp/scoot-after`;
`nix build .#scoot-gpu` → `/tmp/scoot-gpu`):

- `readelf -d /tmp/scoot-after/bin/scoot`: `NEEDED libEGL.so.1`;
  RUNPATH contains `...-libglvnd-1.7.0/lib`. `ldd` resolves
  `libEGL.so.1` to that store path. Default `ldd` (CI pattern
  `^lib(gbm|drm|EGL|GLESv2)` on `$1`) shows only libEGL -- no libgbm.
- `/tmp/scoot-after/bin/scoot --headless ... --renderer gles` →
  `the GLES renderer is up device=/dev/dri/renderD128 software=false`
  (llvmpipe-backed virtio node); session stays up, no panic.
- `ldd /tmp/scoot-gpu/bin/scoot` shows `libgbm.so.1` (→
  `...-mesa-libgbm-26.1.3/lib`), `libEGL.so.1`, `libdrm.so.2`. The
  `-gpu` binary runs headless pixman panic-free and reaches the same
  `GLES renderer is up` line under `--renderer gles`. (Scanout itself
  needs a `--tty` seat: compile-proven here, runtime stays Test 4's.)
- Graceful-error proof (cargo debug binary, libEGL hidden by
  bind-mounting an empty file over
  `/run/current-system/sw/lib/libEGL.so.1` inside `unshare -Urm`):
  `scoot: could not load libEGL.so.1: ... file too short; --renderer
  pixman, the default, needs no GPU at all`, exit 1, zero panic lines.
  Same mask on the pre-fix binary: `Failed to load LibEGL` panic, exit
  134. Unit tests pin both halves (`libegl_loads_where_the_suite_runs`,
  `a_missing_library_is_a_named_error_not_a_panic`).
- `nix flake check`: green on the VM (Linux systems) and on the Mac
  (Darwin; `checks.aarch64-darwin.scoot-modules` built), plus explicit
  `nix build .#checks.aarch64-linux.scoot-modules`. `nix fmt --check`
  over all tracked `.nix` clean.
- Darwin: `nix build .#scoot-gpu` and `.#scoot` both build and run
  (`--version` prints; `--headless` gives the honest Linux-only
  message; `otool -L` shows libSystem only).
- Cargo suite (`.rs` was touched): `cargo nextest run --workspace`
  1317 passed / 6 skipped; `cargo clippy --workspace --all-targets`
  `-D warnings` clean; `cargo fmt --check -p scoot` clean;
  `scripts/smoke-test.sh` 19/19 `ok` on the default build (also 19/19
  on the featured build); cargo `ldd` assertions mirror CI (default:
  no GPU libs; `--features gpu-scanout`: libgbm). Benchmark n/a --
  nothing on a hot path (the probe runs at most once per session
  startup).

## Explicit remainder (stays open by design)

- **Asahi Test 4** (`Asahi.md`): real-GPU scanout, split
  render/display, real performance numbers. The Asahi M2 was not
  available here; nothing above claims it. Its build lines now name
  `nix build .#scoot-gpu`.
- **CI workflows untouched** (`#173` owns CI): `.github/workflows/nix-build.yml`'s
  header still says gpu-scanout "is deliberately NOT built here" --
  stale now that the package exists; flagged for #173, not fixed here.
  Likewise no new CI jobs in this PR per the cost call there.

## Out of scope (per the ticket, and stayed out)

Renderer changes (none), Asahi Test 4 itself, CI workflow changes,
session-command (#171, landed).
