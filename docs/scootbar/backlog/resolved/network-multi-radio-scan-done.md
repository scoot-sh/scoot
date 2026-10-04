---
title: "Network: a second radio's empty scan replaces the real one"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-04"
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

## Resolution (PR #422)

Filtering, not per-interface storage: only the scan target's dump is
queued — the shown interface's (the config's, else the default route's)
where it is a scanning station, else the associated station's, else the
first idle station's so the picker still works off-network. A 32x32
per-interface table was rejected for the memory it pins for a switch a
re-dump serves. AP, AP_VLAN and P2P_GO never qualify (the type comes
from the interface notice's `NL80211_ATTR_IFTYPE`, checked against
linux-headers-7.1); guessing AP-ness from empty caches was rejected
because an idle station's cache is empty too.

Pinned by six scripted-kernel tests (each fails with the fix reverted,
passes with it; the idle-station one passes both as a no-regression
pin): the shown radio dumping first, an AP interface's empty cache, a
dongle plugged in with the popup open, the default route moving, the
shown radio vanishing, plus the iftype parse unit. Full suite green on
the dev VM (nextest 1328 passed, clippy matrix clean, flake loop 20x).

Proved live with `mac80211_hwsim` (radios=3: station associated to
hostapd's TestNet0 with TestNet0/1/2 up, two idle radios, three AP-mode
interfaces): on main the popup lists one wrong network (TestNet1, the
associated missing); on the branch all three with TestNet0 selected.
Ratchet: release file unchanged (2,167,520 B), `.text` +1,856
(+0.11%), idle with network placed 0 wakeups/60 s and level RSS on both
— the `.text` row is reported, not waived.
