---
title: "scootgreet: the login screen, entry, focus chain, layout and Caps Lock, built-in defaults"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-greetd-client, scootui-entry-and-list"
milestone: "M7"
---

# scootgreet: the screen

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#what-the-screen-offers); split in the #579 fix
round. Serves **daily-drive**.

## What to do

- The full-screen `xdg_toplevel` card, holding a username entry, the PAM prompt and its entry,
  and a message line.
- The focus chain.
- The layout name, and the Caps Lock and Num Lock indicators.
- Text scale and high contrast.
- The fingerprint-wait state, with Escape greyed.
- Built-in defaults on any config or look error, with an error line on screen and never an exit.
- The wallpaper (PNG, JPEG), scaled once per output.

## Acceptance

- Fake-greetd screenshot tests per state.
- A bad config shows the defaults plus an error line, and logs in.
- A real login on the dev VM.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The user list, sessions and power (their own tickets), and Nix.
