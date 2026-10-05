---
title: "Per-client allow-list for the virtual-input globals via security-context-v1"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Per-client allow-list for the virtual-input globals via security-context-v1

Filed 2026-10-05 from PR #448's review. Serves **daily-drive**: a user
running wayvnc wants only wayvnc able to drive the session, not every
same-uid process.

## The gap

`[virtual_input] enabled` advertises the virtual-pointer and
virtual-keyboard globals to every same-uid client: any process that can
bind them can type and click as the user
(`crates/scoot/src/compositor/virtual_input.rs`, Gating section;
`site/src/content/docs/scoot/protocols.md` trust note). An allow-list
naming privileged clients would be theatre today -- scoot has no
`security-context-v1` support, so it cannot tell wayvnc from any other
same-uid client -- which is why #448 ships the on/off switch as the whole
gate. The switch answers the network threat (wayvnc listens on TCP), not
the local-client one.

## What to do

Once scoot supports `security-context-v1`, gate the two virtual-input
globals on it: advertise (or accept binds from) only clients carrying a
security context the user allow-listed, keeping the current switch as the
default-deny underneath. Pin with tests: an unlisted same-uid client sees
no globals (or its bind fails) while a listed one drives the session; the
lock-gate and the destroy/disconnect release paths keep working for
listed clients.

## Not in this ticket

Implementing `security-context-v1` itself; changing the switch default;
per-window or per-seat scoping (see `native-remote-scene-streaming.md`
for the fuller remote story).
