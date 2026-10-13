---
title: "scootgreet: the login screen, users, sessions, power, layout and Caps Lock; the go/no-go gate"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-greetd-client, scootui-entry-and-list, scoot-dbus-extract, launch-desktop-index"
milestone: "M7"
---

# scootgreet: the screen

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#what-the-screen-offers). Serves
**daily-drive**.

## What to do

The full-screen `xdg_toplevel` card with the focus chain; users from `/etc/passwd` and
`login.defs` with "Other…"; AccountsService icon files; sessions from `wayland-sessions` through
the desktop-entry parser (moved here into a shared `scoot-xdg` crate, its second consumer);
remembered state; logind power with `Can*` gating and a confirm; layout name and Caps Lock
warning; text scale and high contrast; built-in defaults on any config error; wallpaper
PNG and JPEG scaled once per output.

## Acceptance

The **go/no-go gate** against `greet-baseline`: closure and RSS at most a third of ReGreet's
stack, first frame no later, the PAM matrix on the dev VM (password, wrong password, expired
password change, OTP, info-only). Fake-greetd screenshot tests per state. A bad config shows
the defaults and an error line, never an exit.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The gate table with ReGreet beside it.

## Not in this ticket

Nix, the agreety wrapper.
