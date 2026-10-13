---
title: "scootgreet: packages, site docs, and the scoot-iso target switch with its QEMU test"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-nixos"
milestone: "M7"
---

# scootgreet: packaging, docs and the ISO

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#the-installer-iso). Serves **daily-drive**.

## What to do

- **Packages**: deb, rpm and Arch, with the `scootgreet-greeter` and `scootgreet-fallback`
  scripts and a greetd config example whose `default_session.command` is the wrapper's path. `install.sh` never wires them.
- **The site's greeter section**: the choice of greeter, the recovery path, and the
  screen-reader note.
- **In `scoot-sh/scoot-iso`**:
  - switch the target to scootgreet, measuring the ISO and target closure delta;
  - make `scripts/qemu-test.sh` type into the focused field instead of clicking coordinates;
  - offer ReGreet as the installer's screen-reader choice.

## Acceptance

The packages build, the site builds, and the scoot-iso QEMU test passes with scootgreet on the
target.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The ISO size before and after.

## Not in this ticket

The default flip.
