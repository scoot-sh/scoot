---
title: "scootlaunch: the panel, launching with a token into a systemd scope, icons; the go/no-go gate"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-match-rank, scootui-entry-and-list, scoot-dbus-extract"
milestone: "M7"
---

# scootlaunch: the panel and launching

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#keyboard-first-ux). Serves **daily-drive**.

## What to do

The `overlay` exclusive-keyboard panel with the entry and the virtualized list, keys and
mouse per the design, single-instance toggle, output choice. Launching: the activation
token from the choosing key or button serial (wait for `done`, 200 ms bound), the
environment rules, `StartTransientUnit` into `app-scootlaunch-<id>-<random>.scope`
(confirm the slice systemd's desktop-environment page asks for; bounded 500 ms), the
double-fork fallback, `DBusActivatable` entries. Icons with theme inheritance, PNG, and SVG
behind a default-on `svg` feature (`resvg`) with its size row measured for a waiver ruling.

## Acceptance

The **go/no-go gate**: first frame warm and cold, open RSS and keystroke-to-commit at 300 and
3,000 entries, all at or under fuzzel's in the same harness on the M2, else stop and record
why. A headless test where a mouse-picked entry re-activates an already-running
single-instance stub (the fuzzel regression `activation.rs:81-99` documents). The launched
app shown in its own scope under a user manager; the fallback exercised.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The gate table with fuzzel beside it, the `systemctl --user status` output.

## Not in this ticket

dmenu, agent `--run-id`, windows mode, Nix.
