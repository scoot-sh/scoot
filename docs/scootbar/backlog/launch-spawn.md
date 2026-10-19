---
title: "scootlaunch: launching right (activation token, environment, systemd scope, fallback fork)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M7"
---

# scootlaunch: launching

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#launching-correctly); split in the #579 fix
round. Serves **daily-drive**.

## What to do

- **The activation token**, from the choosing key or button serial; wait for `done` with a
  200 ms bound.
- **The environment rules**: strip the launcher's own token, set the new one, `Path=`, stdin
  from `/dev/null`.
- **The systemd scope**: `StartTransientUnit` into `app-scootlaunch-<id>-<random>.scope`,
  bounded at 500 ms. Confirm which slice systemd's desktop-environment page asks for.
- **The fallback**: a double fork with `setsid`.
- **`DBusActivatable` entries**, through `org.freedesktop.Application` with `activation-token`
  in `platform_data`.

## Acceptance

- A mouse-picked entry re-activates an already-running single-instance stub (headless).
- The launched app shows in its own scope under a user manager.
- The fallback is exercised with no bus.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `systemctl --user status` output.

## Not in this ticket

dmenu mode.
