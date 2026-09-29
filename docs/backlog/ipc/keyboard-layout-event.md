---
title: "Keyboard layout: an IPC query and `subscribe` event, so a bar can show it"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# Keyboard layout in IPC

Filed 2026-09-29 (scootbar research). Serves **daily-drive** (multi-layout
users) and **computer use** (an agent needs to know which layout `msg type`
will produce).

## The gap

A keyboard-layout indicator is one of the most-requested bar modules (Waybar
#66, 44 comments), but **no standard Wayland protocol tells a client the
active layout**, so a bar cannot get it from scoot through the standards it
otherwise uses. Whether scoot already exposes any of it over IPC needs checking
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
