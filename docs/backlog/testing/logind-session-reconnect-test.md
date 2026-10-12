---
title: "Test the seat/VT-switch recovery path on a logind-backed session"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# Test the seat/VT-switch recovery path on a logind-backed session

Filed 2026-10-11. Serves **daily-drive**: most desktops start through logind..

## The gap

The VT-switch recovery work (#546, #551) was proven against a private seatd
on the Asahi M2. A logind session never races for master the way seatd does
(logind claims master on its own fd before it resumes the client), so the same
path may behave differently, or not run at all. Nobody has measured it. The
M2's greeter runs cage+regreet through logind on tty1, so the machine can host
the control.

## What to do

Add a logind-present control to the live matrix: switch away and back under a
logind session and record whether scoot's reconnect path fires, stays quiet,
or misbehaves. File any defect as its own ticket.

## Not in this ticket

Changing the seatd recovery itself.
