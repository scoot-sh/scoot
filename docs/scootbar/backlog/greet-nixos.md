---
title: "scootgreet: NixOS module, Stylix and look, the agreety fallback wrapper, opt-in"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-ui"
milestone: "M7"
---

# scootgreet: NixOS

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#stylix-and-look-integration) and its
[failure modes](../../../dev/design/scootgreet.md#failure-modes-never-strand-a-login). Serves **daily-drive**.

## What to do

`programs.scootgreet` (NixOS) and `programs.scoot.greeter.daemon = "regreet" | "scootgreet"`
(default `"regreet"`). With scootgreet: greetd's command is `scootgreet-session` (cage
without `dbus-run-session`; on a non-zero exit, `exec agreety --cmd <default session>`),
tmpfiles for `/var/lib/scootgreet`, `services.xserver.xkb.*` into the greeter's environment,
the Stylix (`sizes.applications`, `image`, `cursor`) and look rungs per leaf. Each look with a
`regreet.css` gains `greet.toml`.

## Acceptance

`nix/scootgreet-tests.nix` pins the mapping, user wins, `--check`, the wrapper and its fallback,
and today's ReGreet output byte-identical with the default. On the dev VM: a real login; an
injected crash lands on agreety, never a dead VT1; `Ctrl+Alt+F2` reaches a getty.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The crash-fallback transcript and a screenshot of each state.

## Not in this ticket

The scoot-hosted mode, the ISO.
