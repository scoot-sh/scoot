---
title: "Look leaves for mako, wob, fcitx5 and the night light"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
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

## Resolution (2026-10-15)

Landed as documented above, with the restart side of the wob choice: a
unit restart (wob starts in milliseconds and holds no state) beat the
all-styles file (which needs every look's style rendered and named per
message, on both the Nix and the plain path).

`scoot-look` renders `mako`, `wob.ini`, `fcitx5/theme.conf` and
`nightlight` beside `scoot.toml`/`bar.toml`/`render.json`, each
re-checking every interpolated `#rrggbb` past the parse gate, with
golden and hostile-input tests per format. `scoot look apply` renders
the same set into the cache (schema v3), runs the leaf signals after
commit (`makoctl reload`, `ReloadAddonConfig "classicui"` over D-Bus,
`try-restart` of the `scoot-osd` and `scoot-nightlight` units), and
skips them all on private states and before any rollback. No
`[apps.mako]`/`[apps.wob]`/`[apps.fcitx5]` overrides in v1: user
settings win by file order. Nix points each app at the state link and
seeds the declared default at activation (mako and wob exit on a
dangling config, verified live).

Evidence: mako 1.11.0 restyles an on-screen notification across a
`makoctl reload` (screenshot pixels `#2B3648` to `#FCFBFB`); wob 0.16
shows the new palette after a unit restart (pixels `#FFA45C` to
`#FCFBFB`/`#3D579A`); fcitx5 5.1.21 answers the `org.fcitx.Fcitx5
/controller org.fcitx.Fcitx.Controller1 ReloadAddonConfig` call with
exit 0; wlsunset 0.4.0 sets the fragment's temperature through the
wrapper logic (3400 K read, 3500 K fallback). Quoting tests per format
in `scoot-look`; the `shipped` gate pins music-desk's old eval-time
values byte-exact in the renderer. macOS and Linux renders are
byte-identical. `nix/tests.nix` eval asserts and all leaf content
checks pass; the monolithic content run could not go green on the dev
VM (root disk exhausted by concurrent agents, failing in unrelated
power/watcher sections), so CI owns the final full run.

PR: https://github.com/scoot-sh/scoot/pull/632 (open, do not merge: the coordinator gates merges).
