---
title: "scootnotify vs mako A/B: resident PSS, idle wakeups, notify-to-first-frame"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootnotify vs mako A/B: resident PSS, idle wakeups, notify-to-first-frame

Filed 2026-10-21. Serves **daily-drive**. The notify default flip
shipped without an honest headless measurement: no numbers are
published anywhere, and none were claimed.

## The gap

`dev/benchmarks/launcher-ab-2026-10.md` proves the launcher flip with
M2 numbers (N=30/cell, release binaries, same session). The notify
flip has no equivalent: resident PSS, idle wakeups
(`voluntary_ctxt_switches` over 60 s, per dev/agent-pitfalls.md) and
notify-to-first-frame for scootnotify vs mako were never measured
under one harness.

## What to do

Following `scripts/launcher-ab-bench` conventions (sandboxed XDG,
same font, N>=30, floor measured): headless scoot on the M2, one
`Notify` per trial, screenshot-diff poll at ~10 ms for first frame,
PSS while a card shows and at idle, 60 s wakeup count at idle. Write
it up as `dev/benchmarks/notify-ab-2026-10.md`. If either daemon
cannot be driven cleanly headless, say so in the ticket and close it.

## Not in this ticket

Gating the default on the numbers (the flip already shipped; the
escape hatch stays one line away either way). Live-login proofs per
look (covered by the flip ticket's own criteria).
