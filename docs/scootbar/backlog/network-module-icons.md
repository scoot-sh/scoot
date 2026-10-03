---
title: "Network module: icon keys per state, and an icon-only mode"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
([popup-network-list](popup-network-list.md)); nl80211/VPN detection
changes; layout beyond prepending/only-icon.
