---
title: "scootgreet: logind power actions with `Can*` gating and a confirm"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-ui, scoot-dbus-extract"
milestone: "M7"
---

# scootgreet: power

Filed 2026-10-12 in the #579 fix round, split out of `greet-ui`. Serves **daily-drive**.

## What to do

Suspend, Reboot and Power Off through `org.freedesktop.login1.Manager` with
`interactive = false`. `CanPowerOff`, `CanReboot` and `CanSuspend` gate which buttons show. Each
action has a confirm step, and config can override any of them with an argv.

## Acceptance

- Against a real logind on the dev VM: the buttons shown match `Can*`.
- Suspend works.
- The power buttons stay usable while a fingerprint prompt blocks greetd.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
