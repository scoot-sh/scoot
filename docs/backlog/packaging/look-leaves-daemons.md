---
title: "Look leaves for mako, wob, fcitx5 and the night light"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# Look leaves for mako, wob, fcitx5 and the night light

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md) (split from
the third-party leaves). Serves **daily-drive**.

## The gap

mako (`notifications-home.nix:316`), wob (`audio-home.nix:417`), fcitx5
(`inputmethod-home.nix:203-204`) and the night temperature
(`nightlight-home.nix:247,260-261`) are themed by store files or unit
arguments; a swap reaches none of them, and a non-Nix box gets none.

## What to do

Move each mapping into `scoot-look`'s renderer with per-format quoting
(section 3.5), and point each app at the link:

- mako: `include=` the link's file; `makoctl reload` on swap.
- wob: restart the unit on swap (or one `wob.ini` with every look's
  `[style.X]`, selected per message; measure which is simpler).
- fcitx5: D-Bus `ReloadAddonConfig "classicui"` (`fcitx5-remote -r` does not
  restyle it).
- nightlight: the temperature read from the look at unit start; restart on
  swap.
- The user's per-key settings keep winning (`// notif.settings`-style).

## Evidence to record

Per app, a swap observed on the dev VM with the version; a quoting test per
format.

## Not in this ticket

GTK/Qt, launchers, terminals.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
