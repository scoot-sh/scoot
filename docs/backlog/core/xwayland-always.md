---
title: "Always compile in xwayland: pure-Rust x11rb costs no link-time dependency"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Always compile in xwayland: pure-Rust x11rb costs no link-time dependency

Filed 2026-10-10 (spike `runtime-gbm` follow-up). Serves **daily-drive**:
today X11 support is a second build axis (`scoot-xwayland`,
`scoot-gpu-xwayland`) a user must predict the need for; if it costs no
link-time dependency it should be compiled in and gated at runtime like
everything else.

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
2. If the delta is accepted, compile the feature in unconditionally (or as
   a default-on feature), keeping every runtime gate exactly as is, and
   collapse `flake.nix` `scoot-xwayland` / `scoot-gpu-xwayland` into their
   base packages (keeping the `PATH` wrapper for the Xwayland binary).
3. Update `install.md#which-build-do-i-need`: the X11 axis goes away with
   the GPU axis (see `./runtime-gbm-productionise.md`), leaving one
   package plus the trust decision, which stays in the config, not the
   download.

## Not in this ticket

Removing the `--xwayland` runtime gate: opt-in stays. Bundling an Xwayland
binary into the closure: it stays a `PATH` lookup with a loud Wayland-only
fallback.
