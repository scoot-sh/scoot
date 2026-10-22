---
title: "Deferred: hyprland-global-shortcuts-v1 and xx-hotkey-v1"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Deferred: hyprland-global-shortcuts-v1 and xx-hotkey-v1

Filed 2026-10-22 from W43. Records the two protocol paths scoot
deliberately did *not* take for global shortcuts, and what would make
either worthwhile. Revisit when the portal backend (stage 2) exists and
real clients report gaps -- not before.

## The options

- `hyprland-global-shortcuts-v1` (compositor-side Wayland protocol):
  compositor exposes `register_shortcut(id, app_id, description,
  trigger_description)` and sends `pressed` / `released` events.
  xdg-desktop-portal-hyprland reuses exactly this: its portal backend is
  a thin client of the compositor protocol, so the compositor owns the
  grab and the backend owns the portal surface.
- `xx-hotkey-v1` (wayland-protocols MR 525): the emerging standard
  Wayland protocol for the same job. Experimental; KWin has stated it
  will not implement it, so it is not the standard its number suggests
  yet.

## The tradeoff

The XDPH reuse option: implement `hyprland-global-shortcuts-v1` in
scoot and run (or fork) xdp-hyprland's backend against it instead of
writing a scoot-owned backend. Cheaper portal surface, but it binds
scoot to a Hyprland-namespaced protocol for a cross-desktop feature and
inherits xdp-hyprland's process and release cadence. The project rule
(standard over bespoke, scoped sensibly) currently favors the
scoot-owned `impl.portal.GlobalShortcuts` backend: the compositor side
is already portal-shaped (see `dev/design/global-shortcuts.md`), and
the portal interface is what apps actually target.

Revisit when one of these is true: `xx-hotkey-v1` stabilizes with
multi-compositor adoption (then prefer it over the bespoke grab);
xdp-hyprland's backend becomes reusable without the compositor protocol
(then re-cost the reuse); or real portal clients on scoot report gaps
the owned backend cannot close.

## Not in this ticket

The stage-2 backend itself (separate ticket). No code here.
