---
title: "Network module: link state, WiFi name and signal, click to pick"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
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

## What landed (PR #377, 2026-10-02)

The `network` module (`crates/scootbar/src/modules/network/`, Cargo
feature `network`, on by default), reference in
[cli.md](../../cli.md#network). Two netlink sockets on the poll loop, no
daemon and no new dependencies: rtnetlink for links, addresses and the
default route; nl80211 (hand-rolled generic-netlink framing) for the
wireless list, the associated SSID, the signal and the cached scan. The
shown interface is the config's `interface` or the default route's (v4
before v6), tracked by index across renames; states are ethernet (the
name), WiFi (SSID and 1–4 bars), VPN, and `offline` in `warn`. A click
(or the `menu` action) spawns `menu-command` with the cached scan's SSIDs
on stdin; `show-ssid = false` hides the SSID in the view and `query`.
Signal re-reads on a 10 s timerfd armed only while WiFi is shown —
`SET_CQM` is refused (`EPERM`, measured on brcmfmac, pinned by the live
test) — while connect/disconnect/roam stay events (roam also re-reads
the interface, so a stale scan cache cannot leave the old SSID shown).

Evidence: `cargo nextest run -p scootbar` 875 pass on the dev VM (with
volume and microphone merged underneath);
clippy `-D warnings` clean on the default, minimal and each-module-alone
builds; `fmt --check` clean; the feature-matrix `--bin` runs all pass.
Live on the dev VM (eth0): `ethernet` in ~100 ms, 1 wake in 20 quiet
seconds. Live on the Asahi box (wlan0, associated): `Wimbly` with bars
in ~106–120 ms, 2 wakes in 20 quiet seconds (the timer's own), a 492-byte
scan dump captured into the fuzz corpus. Review of the lane's first
draft caught a byte-swapped `ifi_type` (broke tun/tap/PPP VPN detection),
a short `GETLINK` body, and an unbounded dump-retry spin, all fixed with
live pins. Review round two caught an off-by-one multicast mask (group
ids are 1-based, the bind bit is `id - 1` — the socket had joined the
wrong groups, so roam notices never arrived; proven by a rescan's
`TRIGGER_SCAN` arriving 19 ms after the fix, and pinned by a live test
that reads the subscription back from `/proc/net/netlink`), a
`show-ssid = false` hole (the picker piped real SSIDs; it now refuses
with a log line), a two-radio scan-list mixup, an unusable default route
hiding a working one, and a quiet-test bound the module's own signal
tick legitimately breaks (it now pins state/SSID stillness instead).
A bare scan-completion notice re-dumps nothing anymore: it carries no
networks, and the picker re-dumps when it opens. Not run live: forced
disconnect/roam (would drop the test session's own ssh), suspend/resume,
the full bench-ratchet run (module-level numbers published in
[lightest.md](lightest.md) instead).

The module API needed no changes for this module (`sources`, `on_ready`,
`view`, `value`, `invoke` as they stand) — but it stays unfrozen until
volume, the other exercising module, lands too.
