---
title: "Probe: does scoot send wl_keyboard.leave when a client destroys its only surface"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Probe: does scoot send wl_keyboard.leave when a client destroys its only surface

Filed 2026-10-18. Serves **computer use (stuck focus state delivers phantom input to the next focus)**.

## The gap

While testing #670 the implementer saw no `wl_keyboard.leave` after a client destroyed its only surface under headless scoot. R-670 traced the path statically (`toplevel_destroyed` -> `remove_window` -> `set_focus(None)` -> Smithay `leave`) and found no reason it would be skipped, but nobody has a repro. Worst case: a client-side `focused` flag and a live key repeat on a surface-less client.

## What to do

Write a minimal headless probe: map a surface, take keyboard focus, destroy the surface (not the connection), and assert `leave` arrives. If it does not, find where focus is dropped silently and fix it in scoot; if it does, close this with the test kept.

## Not in this ticket

The key-repeat logic itself, which disarms on `leave` already.
