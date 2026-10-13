---
title: "Third-party leaves rendered from tokens and read through the look link"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# Third-party leaves rendered from tokens and read through the look link

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

Every third-party theme today is a store file or a store path baked into a
script: mako (`notifications-home.nix:316`), swaylock run with
`-C ${swaylockConfig}` (`idle-home.nix:83`), fuzzel flags inlined
(`keys-home.nix:72-79,139-141`, `apps-home.nix:67-73`), wob
(`audio-home.nix:417`), fcitx5 (`inputmethod-home.nix:203-204`), the night
temperature in a unit's arguments (`nightlight-home.nix:247,260-261`), and
foot/starship/Helix/btop as static example files
(`theme-home.nix:657-678`). A swap reaches none of them, and a non-Nix box
gets none of them.

## What to do

- Move each mapping into `scoot-look`'s renderer (one implementation for
  Nix and non-Nix) and point each app at the link, per the liveness table:
  - mako: an `include=` of the link's file; `makoctl reload` on swap
    (re-styles notifications on screen).
  - swaylock: `-C` the link's file (next lock).
  - fuzzel: `--config` the link's file instead of inlined flags (next
    launch).
  - wob: restart the unit on swap (or one `wob.ini` holding every look's
    `[style.X]`, selected per message: measure which is simpler).
  - fcitx5: D-Bus `ReloadAddonConfig "classicui"` on swap
    (`fcitx5-remote -r` does not restyle it).
  - nightlight: the temperature read from the look at unit start; restart
    on swap.
  - foot: `[colors-dark]`/`[colors-light]` from `[terminal]` (foot 1.28 has
    no `[colors]`); new terminals only. A live OSC push into running ptys is
    a research item: it can interleave with a program's own output.
  - Helix: `SIGUSR1` to the user's own `hx` processes; btop: `SIGUSR2`.
    Both are their documented reloads; opt-out per app.
  - starship: next prompt, nothing to do.
- Static leaves (`apps/`) only from trusted looks (design doc §8.3).

## Evidence to record

Per app: a swap observed live (or at next launch) on the dev VM, with the
version, and the user's own per-key settings still winning
(`// notif.settings`-style precedence preserved).

## Not in this ticket

GTK/Qt ([`look-toolkit-live`](look-toolkit-live.md)).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
