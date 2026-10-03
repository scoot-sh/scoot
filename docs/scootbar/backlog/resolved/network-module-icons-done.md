---
title: "Network module: icon keys per state, and an icon-only mode"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# Network module: icon keys per state, and an icon-only mode

Filed 2026-10-03 from gh issue #379. Serves **daily-drive**: a network
indicator that is an icon like the clock, volume and `button` modules can
be — a wired/Wi-Fi glyph that changes with state, details in the tooltip.

## The gap

The `network` module always shows text: interface name for Ethernet,
SSID+bars for Wi-Fi, `VPN`, `offline`. Its table takes `interface`,
`show-ssid` and `menu-command`, but none of the icon keys the clock and
volume take (`icon`, `icon-path` with `icon-viewbox`, `icon-image`).
Checked 2026-10-03: `crates/scootbar/src/modules/network/` has no `icon`
reference at all, while `crates/scootbar/src/modules/volume/mod.rs`
parses a static `icon` plus four level icons (`mod icons`). So there is no
way to make it icon-only, or to put an icon in front of its text. Seen on
scoot `ebe99d1` (#377), vfkit VM, Ethernet only.

## What to do

Roughly (open to what fits the module):

- The same icon keys as clock and volume (`icon`, `icon-path`,
  `icon-image`).
- One icon per state, the way volume picks one of four by level:
  `icon-ethernet`, `icon-wifi` (optionally per signal bar), `icon-vpn`,
  `icon-offline`, falling back to `icon`. Volume's "a static `icon`
  replaces all four" rule could carry over.
- `show-text = false` (or `format`) to draw only the icon, with the
  current text moved into the tooltip. Tooltips now exist (#392), and
  `query` already carries the details.

Pin per-state fallback order and the `query` shape in tests, following the
volume module's contract tests.

## Not in this ticket

The native WiFi list popup
([popup-network-list](resolved/popup-network-list-done.md)); nl80211/VPN detection
changes; layout beyond prepending/only-icon.

## Resolution (2026-10-03, PR #401)

Landed as `feat(scootbar): network module icon keys per state and icon-only mode` (`c82cb983`). Static `icon`/`icon-path`/`icon-image` plus per-state `icon-ethernet/wifi/vpn/offline` (per-state wins, falls back to `icon`); `show-text = false` draws icon-only with text in tooltip; `query` shape unchanged. Review: no blocking findings (2 CI bin failures reproduced identically on clean base — media timing race + media contract culprit, both pre-existing; the one CI `keep_tests` failure on the PR flaked green on rerun). CI green.
