---
title: "A shared D-Bus client for the shell"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# A shared D-Bus client

Filed 2026-09-29. Serves **daily-drive**: it is the largest single saving
across the shell if it is small, and the largest cost if it is not.

Notifications (`org.freedesktop.Notifications`), the system tray
(StatusNotifierItem), NetworkManager or iwd, logind and UPower all need D-Bus.
Building it once, in its own small crate, is cheaper than each consumer
pulling `zbus` and its async runtime.

The spike below is this entry's own first task, not a blocker: nothing waits on
it before the entry can start.

## The spike, first

`zbus`, libdbus bindings and a hand-rolled minimal client (auth EXTERNAL,
`Hello`, `RequestName`, method calls, signal match rules). Measure a process that
owns a name and receives a signal: idle RSS, binary size, wakeups. This spike moved
here from [baselines-and-spikes](baselines-and-spikes.md) so it is decided when the
first consumer needs it, not before.

## What to do

Take the spike's result. If a hand-rolled client wins, it needs only: the
session and system bus connect, EXTERNAL auth, `Hello`, `RequestName`, method
calls and replies, signal match rules, and a marshaller for the handful of
types the consumers use, all driven from the same `poll(2)` loop (its fd is a
source, not a thread). If `zbus` or libdbus wins by enough on correctness
risk, use it and record the measured cost.

## Guard rails

A hand-rolled bus client parses untrusted bytes from a same-user peer: fuzz
the message parser, cap message and array sizes, and treat a malformed message
as a dropped connection, never a panic. Licence-check anything pulled in.

## Done when

One consumer (start with [scootnotify](scootnotify.md) or the
[tray](tray.md)) runs on it and its idle cost is recorded.
