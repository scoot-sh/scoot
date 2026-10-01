---
title: "Network module: link state, WiFi name and signal, click to pick"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M5"
---

# Network module

Filed 2026-09-29. Serves **daily-drive**; with [volume](volume-module.md) the
hardest test of the module API.

## Sources

- **Link and address state**: rtnetlink (`RTM_NEWLINK`, `RTM_NEWADDR` groups)
  on a netlink socket in the `poll` loop; pure events, no polling.
- **WiFi SSID and signal**: nl80211 over generic netlink (resolve the family id,
  subscribe to the `mlme`/`scan` multicast groups, query the current
  connection). Or the network daemon's D-Bus (NetworkManager, iwd) through the
  [shared client](dbus-client.md).
- **Signal strength changes have no natural event.** Options to measure:
  connection-quality (CQM) RSSI thresholds where the driver supports them, a
  slow timer only while the module is visible and connected, or showing
  coarse bars from the association event only. Pick by measured wakeups.

## What to build

- States: ethernet up, WiFi connected (SSID, bars), disconnected, VPN interface
  present; a class per state.
- Click opens a picker: in the interim, a dmenu-style launcher fed from the
  daemon's scan list; natively via [popups](popups.md) later. Connecting is the
  network daemon's job; the bar only asks it.
- **Privacy option**: hide the SSID (`show-ssid = false`), because the bar is
  visible in screenshots and to an agent's `query`.
- `Unavailable` where there is no network hardware or no supported daemon.

## Edge cases

Interface renamed, several interfaces at once (choose by config or show the
default route's), a link flapping at a high rate (coalesce), no permission for
nl80211, suspend/resume with stale state.

## Done when

Connect, disconnect and roam are reflected within a frame on real hardware,
signal strength's update strategy is measured and recorded, and idle wakeups
are only real network events.
