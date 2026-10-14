---
title: "Always compile in xwayland: pure-Rust x11rb costs no link-time dependency"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Always compile in xwayland: pure-Rust x11rb costs no link-time dependency

Filed 2026-10-10 (spike `runtime-gbm` follow-up). Serves **daily-drive**:
today X11 support is the last remaining build axis (`scoot-xwayland`,
since PR #588 collapsed the GPU axis) a user must predict the need for;
if it costs no link-time dependency it should be compiled in and gated
at runtime like everything else.

## The gap

Unlike `gpu-scanout` (a real link-time libgbm dependency), the `xwayland`
feature pulls pure-Rust `x11rb` -- the X protocol over a socket -- plus
`encoding_rs` and `scopeguard` (`crates/scoot/Cargo.toml`). The crate
comment already states the `ldd` pair shows no new DT_NEEDED entry on
either side. The remaining costs are the ones this ticket measures, not
assumes: binary size delta, and the runtime `Xwayland` binary on `PATH`
(the session knob `--xwayland` / `[xwayland] enabled` stays opt-in either
way -- sessions stay byte-identical by default -- and the trust note in
`site/src/content/docs/scoot/xwayland.md` still applies: any X client can
keylog/snoop by design).

## What to do

1. Measure the binary size delta of `+xwayland` on the default build and
   record it here.
2. If the delta is accepted, compile the feature in by default (a
   default-on feature), keeping every runtime gate exactly as is, and
   collapse `flake.nix` `scoot-xwayland` into `scoot` (keeping the `PATH`
   wrapper for the Xwayland binary; `scoot-gpu-xwayland` is already gone,
   collapsed in PR #588).
   The feature stays disableable: the CPU-only minimal build
   (`cargo build --no-default-features`, see
   `../resolved/runtime-gbm-productionise-done.md`'s stage C gate) excludes XWayland, so
   "compiled in by default" must mean default-on, never unconditional —
   no X11 code, no extra crates, no `Xwayland`-binary expectation in the
   minimal build.
3. Update `install.md#which-build-do-i-need`: the X11 axis goes away
   (the GPU axis already did in
   `../resolved/runtime-gbm-productionise-done.md`), leaving one
   package plus the trust decision, which stays in the config, not the
   download. The minimal build gets its own documented command or
   package there too.

## Not in this ticket

Removing the `--xwayland` runtime gate: opt-in stays. Bundling an Xwayland
binary into the closure: it stays a `PATH` lookup with a loud Wayland-only
fallback.

## Measured (stream X, 2026-10-14, Asahi M2, release `lto = "fat"`)

- Stripped: default 7,037,240 B → `+xwayland` 8,090,328 B
  (**+1,053,088 B, +15.0%**).
- Unstripped: default 9,420,576 B → `+xwayland` 10,878,224 B
  (**+1,457,648 B, +15.5%**).
- `ldd` sonames and `readelf -d` NEEDED entries byte-identical (9 libs) —
  no new DT_NEEDED, as expected for pure-Rust `x11rb`.
- Startup (`scoot --headless -- true`, ms to `scoot is up`, 10 runs each):
  default mean 158.4 ms, `+xwayland`-compiled-but-disabled mean 159.8 ms
  (noise).
- `--no-default-features` release builds, same size/ldd as the old default,
  `cargo tree --target all --edges normal -i x11rb` clean.
- Loud fallback re-verified live: `--xwayland` with no binary on `PATH`
  logs `` `Xwayland` must be on PATH `` plus the searched `PATH` and runs
  Wayland-only.

## Done in stream X (2026-10-14, PR `feat/xwayland-default`)

`xwayland` is now a default feature of the `scoot` crate
(`crates/scoot/Cargo.toml`: `default = ["xwayland"]`); every runtime gate
unchanged (opt-in by flag/config, byte-identical sessions by default).
New unit pin: `a_disabled_session_spawns_nothing_and_opens_no_x_socket`.
Docs in the same PR: `site/.../scoot/xwayland.md` (always compiled in;
enabling needs the binary), one-line `protocols.md` fix, CHANGELOG.

Left for stream G (owns `flake.nix`/builds/install): collapsing
`scoot-xwayland` / `scoot-gpu-xwayland` into `scoot` / `scoot-gpu`
(keeping the `PATH` wrapper for the Xwayland binary), the four crane
`deps*` artifacts with them, `nix-build.yml`'s xwayland package builds,
`ci.yml`'s `--features xwayland` pins (now redundant but green), and
`install.md#which-build-do-i-need` + `builds.md`.
This ticket stays open until G lands that collapse.
