---
title: "scootnotify: cards, the stack, lock and idle rules, outputs"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-server, scootui-extract-wayland"
milestone: "M7"
---

# scootnotify: popups

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#behavior-on-screen). Serves **daily-drive**.

## What to do

The card and stack in the design on one on-demand `overlay` layer surface (input region
the cards' union), pointer actions with `ActivationToken` before `ActionInvoked`, expiry
started on the first frame callback that presents a card, hover pause, idle pause through
`ext-idle-notify` v2 input idle, `output = focused|compositor|NAME` with the on-demand
foreign-toplevel snapshot, hotplug, zero outputs. Config `notify.toml` (style, colors,
placement, timeouts); `msg reload` is in `notify-control`, but the reload path (re-theme in
place) is built here.

## Acceptance

Headless-harness tests: pixels per card state (test font), click invokes with the token
first, a card arriving while locked is not drawn and its clock does not start until after
unlock, output unplug moves the stack, the surface and buffers gone when empty (fds and maps
checked). Zero allocation per repaint. Idle 0 wakeups with no cards.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The ratchet rows from the design's budgets table against lean mako.

## Not in this ticket

Control socket, DND, rules, history, the bar.
