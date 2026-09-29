---
title: "Data-source modules: window title, battery, volume, network, brightness"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock, pointer-and-interactions"
---

# Data-source modules

Filed 2026-09-29. Serves **daily-drive**. One PR per module, each measured
against the [release gate](lightest.md); the rule for all of them is an fd the
loop polls, or nothing. Freeze the module API only after volume and network
have exercised it: they are the two hardest.

| Module | Source | Interactions | Open question |
|---|---|---|---|
| window title | foreign-toplevel (the identifier carries the `scoot msg windows` id) | click focuses via `activate` | which output's focused window; truncation |
| battery | netlink uevents, sysfs read on change | none by default | some drivers emit no event on capacity change; if so a slow timer **only while discharging**, measured |
| volume | PipeWire's PulseAudio-compatible protocol, subscribed to sink events | scroll raises/lowers, click mutes, right-click opens a mixer | interim is `exec` around `pactl subscribe`; native client is a measurement against it |
| network / WiFi | rtnetlink link state, nl80211 events | click opens a picker (see [popups](popups.md)) | signal strength may have no event; decide between a CQM threshold, a slow timer while visible, or omitting it |
| brightness | udev or inotify on sysfs | scroll adjusts | permissions for writing the value |

Each: `Unavailable` (and zero cost) when the hardware or service is absent;
a state class (`warn` at low battery, `muted`); a `query` entry for agents; a
test with fake events through the module harness; docs. The volume and network
modules may need the [shared D-Bus client](dbus-client.md) if PipeWire or
NetworkManager is reached that way.

## Done when

Each module is listed, measured and documented, and none adds an idle wakeup
its source does not justify.
