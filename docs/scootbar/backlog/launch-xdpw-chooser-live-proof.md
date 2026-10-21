---
title: "scootlaunch: live-cast proof for the xdpw output chooser"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch: live-cast proof for the xdpw output chooser

Filed 2026-10-21. Serves **daily-drive**. Since the launcher-default flip,
`capture.chooser = "fuzzel"` (the name is unchanged) runs scootlaunch's
`--dmenu` behind `scoot-screencast-chooser`. The contract it needs (lines
on stdin, pick on stdout, nonzero exit on cancel) is unit-tested and
eval-pinned, but no live ScreenCast has run through it yet.

## The gap

Everything else the flip touched has a live proof (bench M5 pick path,
headless `--no-run-if-empty`, stub-driven picker flows). The chooser path
(xdpw spawning the wrapper, output names in, selection out, cancel
behavior) has never run against a real portal cast.

## What to do

- On the M2 dev setup with two outputs: start a ScreenCast (Meet, OBS, or
  `xdg-desktop-portal-wlr` test client), pick an output through the
  scootlaunch chooser, verify the cast carries that output; cancel and
  verify nothing casts.
- If it misbehaves, either fix scootlaunch's dmenu-for-xdpw shape or pin
  that chooser back to fuzzel with the reason here.

## Not in this ticket

The chooser name (`"fuzzel"`) staying put: renaming it would churn every
existing config for no behavior change.
