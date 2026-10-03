---
title: "A native WiFi list in a popup, where the dmenu picker stops being enough"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# A native WiFi list in a popup

Filed 2026-10-02, from [popups](resolved/popups-done.md). Serves **daily-drive**
(the WiFi list, without a launcher installed).

## The gap

[Popups](resolved/popups-done.md) landed with the volume slider as their one
consumer, so the widget set has text, a slider and buttons, and a *list* is a
column of buttons (one `selected`). The network module's picker is still the
interim path: `network.menu-command` is fed the scan's SSIDs on stdin and does
the connecting itself. That is deliberate. A list of up to 32 networks needs
what the popup does not have yet, and building it without the consumer
would be guessing.

## What to do

- **Scrolling** in a popup taller than the output allows (a wheel over the
  popup, a clamp to the output's height from the positioner's constraint), and a
  row cut with an ellipsis, as the window title's is.
- **Selecting a row closes the popup** (a `closes` flag on a button), and runs
  an action on the network module that takes the choice: an SSID is text, and
  an action takes one whole number, so the action is `connect N` (the index
  into the scan the popup was opened with, checked against the scan as it is
  then), spawning a configured `connect-command` with the SSID as its last
  argument, never through a shell.
- **Opt-in**, as the volume popup is: `on-click = "popup"` on the network
  module, with `menu-command` left as it is, so the dmenu path does not regress.
- A password prompt is out of scope (the command does that).

## Not in this ticket

Keyboard navigation of a list (arrows, Enter), which the popup grab already
gives the popup the keyboard for: its own entry once a list exists.
