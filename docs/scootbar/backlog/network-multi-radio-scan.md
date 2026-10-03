---
title: "Network: a second radio's empty scan replaces the real one"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Network: a second radio's empty scan replaces the real one

Filed 2026-10-03, from the independent review of PR #403 (the native
WiFi list). Serves **daily-drive**: on a machine with more than one
wireless interface (a USB dongle beside the built-in card, a hotspot
interface, a phone tether), the picker and the popup list can show no
networks at all while the card in use has a full scan.

## The gap

The network module dumps each wireless interface's cached scan
(`NL80211_CMD_GET_SCAN`): every fresh wireless interface queues a
`Scan(ifindex)` dump (`queue_scan`), and `pump_genl` resets the stored
scan (`scan_of`, `scan_n`) whenever a dump's interface differs from the
last one's. So the **last dump to finish** defines the list, whatever
interface it came from.

Measured by the review on the dev VM with `mac80211_hwsim`: access-point
interfaces `wlan0`, `wlan0_0`, `wlan0_1` and a station `wlan1`. strace at
start-up showed `GET_SCAN` dumps for ifindex 3, 4, 6 and 7, with
`wlan1`'s reply carrying three real BSS entries, yet `invoke network
popup` refused ("no networks seen yet") because the access points' empty
caches dumped after it. Re-creating `wlan1` at a higher ifindex, so its
dump ran last, made the list appear. Not a #403 regression: the dmenu
picker reads the same cache and has the same problem.

## What to do

- Keep the scan of the interface the module shows (the default route's,
  or `network.interface`), or of the associated station interface, and
  ignore the others' dumps; or keep per-interface scans and pick one.
  Decide which with the multi-interface cases below.
- An interface in AP mode has no useful scan: skip it.
- Pin with tests: two radios with the shown one dumping first; a dongle
  plugged in while the popup is open; the shown interface changing (the
  default route moving to the dongle); the shown interface vanishing.

## Not in this ticket

Merging scans across radios into one list (a network seen by both),
and choosing which radio a `connect` uses: the command's business.
