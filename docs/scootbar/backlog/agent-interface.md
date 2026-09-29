---
title: "Agent interface: `query`, `invoke`, `layout` and `subscribe` on the bar's socket"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "pointer-and-interactions"
milestone: "M4"
---

# Agent interface

Filed 2026-09-29. Serves **computer use** (the reason to make the bar
machine-readable) and daily-drive (scripts). The bar's socket is separate from
scoot's IPC (`README.md`), so these live in `scootbar msg`.

An agent driving a desktop should not have to OCR a screenshot to read the
battery, or hunt pixels to press mute.

## Requests

- **`query [ID]`**: each module's state as JSON: id, output, text, class,
  value where it has one (percent, muted, SSID, active workspace). One request,
  bounded reply. Defined in [config-cli-and-reload](resolved/config-cli-and-reload-done.md);
  this entry adds the fidelity guarantees below.
- **`invoke ID ACTION [ARG]`**: run a module's action exactly as its click or
  scroll binding would (`toggle-mute`, `raise 5`), with no pointer involved. It
  goes through the same code path as `on_input`, so what an agent does and what
  a user does cannot diverge. An unknown module or action is a named error.
- **`layout`**: each module's rectangle in output coordinates and the output's
  origin, so an agent that prefers to click can aim `scoot msg` pointer
  injection at real coordinates. Reflects the last drawn layout, not a guess.
- **`subscribe [KIND...]`**: dedicate a connection to change events (`module`
  updates, `output` added or removed), following scoot's subscribe rules
  (`docs/ipc.md#events`): named kinds only, a subscribed connection serves no
  further requests, a subscriber that stops reading is disconnected, never
  buffered. Coalesced to the frame rate.

## Fidelity rule

`query` and `layout` must agree with what a screenshot shows, or the bar is
lying to an agent. Pin it with a test that compares `query` text and `layout`
rects against a headless-scoot screenshot.

## Bounds

Reply size caps, subscriber count cap, and zero cost with no subscriber (one
branch), as in [robustness-and-limits](robustness-and-limits.md).

## Done when

An agent reads every module's value and presses a button through the socket,
and the fidelity test passes on two outputs at two scales.
