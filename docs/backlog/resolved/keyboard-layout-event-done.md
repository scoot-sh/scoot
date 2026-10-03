---
title: "Keyboard layout: an IPC query and `subscribe` event, so a bar can show it"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# Keyboard layout in IPC

Filed 2026-09-29 (scootbar research). Serves **daily-drive** (multi-layout
users) and **computer use** (an agent needs to know which layout `msg type`
will produce).

## The gap

A keyboard-layout indicator is one of the most-requested bar modules (Waybar
#66, 44 comments), but **no standard Wayland protocol reports the active layout
to an unfocused client**: `wl_keyboard` sends the keymap and the modifier group
only to the client that has keyboard focus, which a bar never does (it takes no
keyboard). So a bar cannot get it from scoot through the standards it otherwise
uses. Whether scoot already exposes any of it over IPC needs checking
(`msg type` resolves keys through the keymap probe, `msg-type-dead-keys-compose-done.md`,
so the state exists inside scoot).

## What to do

- A `keyboard` field in a query, and a `keyboard` kind on `subscribe`
  (`docs/ipc.md#events`) carrying the active layout's name and index, sent when
  it changes, coalesced.
- Follow the subscribe rules already there: a named kind, no cost with no
  subscriber, a slow subscriber dropped and never buffered.
- Say how a layout switch is triggered (a config bind, `xkb` options) so the
  bar's click action has something to call, or leave the click out.

## Not in this ticket

Layout switching UI, per-window layouts.

## Resolution (2026-10-03, PR #404)

Landed as `feat(scoot-ipc,scootctl,scoot): keyboard layout query and subscribe event` (`4f9160c7`). Read from Smithay seat-keyboard xkb state (effective group + layout name); no switch mechanism exists, so read-only with no click action. Protocol 5→6 (new reply variants); detection in `State::key_with` funnel; zero-cost gate with no subscriber; shared emit tail. Pinned by 7 new tests; bench shows no hot-path regression. Review verified the bump necessity + old-client degradation + version sites; empty findings. CI green.
