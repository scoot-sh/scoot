---
title: "Tooltips: a module's `tooltip` shown after a hover delay"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Tooltips

Filed 2026-09-29. Serves **daily-drive**: the full window title, the battery
time, the SSID behind an icon.

A module's `View` already carries an optional `tooltip` string
([module API](resolved/module-api-and-clock-done.md)). Showing it is a popup without a grab,
anchored under the module, shown after a delay and dismissed on leave.

## Cost discipline

The delay is a timerfd armed **only while the pointer rests on a module that has
a tooltip**, and disarmed the moment it leaves: no timer at all otherwise.
The surface exists only while shown. No tooltip is ever drawn for a module
with none.

## Details

Position through the popup positioner so it constraint-adjusts at screen edges
(`popup-constraint-adjustment-done.md`); text wraps at a maximum width; a
tooltip never takes the keyboard or steals a click. It also updates in place
if its module's text changes while shown (a clock tooltip), with damage limited
to the popup.

## Done when

Hover shows and leave hides on headless scoot, no timer is armed off a
module, and a tooltip at the screen edge stays on screen.
