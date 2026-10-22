---
title: "Portal global-shortcuts backend (stage 2: impl.portal.GlobalShortcuts v2)"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Portal global-shortcuts backend (stage 2: impl.portal.GlobalShortcuts v2)

Filed 2026-10-22 from W43. Serves **daily-drive**: media keys, push-to-talk
and record toggles that work while another app holds focus -- the class of
bind scoot's own `[binds]` cannot express (those run compositor actions,
not other apps' shortcuts). Stage 1 (compositor side: registry, grab,
`scoot msg shortcut-*`, `shortcuts_changed` events) landed first, so this
is a backend over an already-tested grab.

## The gap

There is no stable Wayland protocol for global shortcuts (see the deferred
ticket): the standard apps target is the xdg-desktop-portal D-Bus
interface `org.freedesktop.portal.GlobalShortcuts` v2 (`CreateSession`,
`BindShortcuts`, `ListShortcuts`, `ConfigureShortcuts`; signals
`Activated` / `Deactivated` / `ShortcutsChanged`), served by a backend
implementing `org.freedesktop.impl.portal.GlobalShortcuts` v2
(docs: flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.GlobalShortcuts.html).
No backend serves it under scoot today, so portal clients (OBS, chat apps)
have no shortcuts. Backends elsewhere: xdp-hyprland talks
`hyprland_global_shortcuts_v1`; xdp-gnome and xdp-kde need their own shell
services; xdp-wlr has none.

## What to do

- New binary crate `scoot-globalshortcuts-portal` (name TBD): a
  scoot-owned portal backend implementing impl.portal.GlobalShortcuts v2
  over `scoot-dbus` or zbus. `[[package]]` entry in `release-plz.toml`
  with its own `git_tag_name`, a `crates/<name>` path filter in `ci.yml`
  classify, `.portal` file (`UseIn=scoot`, `XDG_CURRENT_DESKTOP=scoot`)
  installed to `/usr/share/xdg-desktop-portal/portals/`, the
  `org.freedesktop.impl.portal.GlobalShortcuts` D-Bus service file, Nix
  module wiring, and deb/rpm/arch entries mirroring `scootnotify`'s unit
  + activation files. See `dev/design/global-shortcuts.md` for the
  compositor-side contract (binding table keyed by
  `(app_id, shortcut_id)`, user owns assignment, compositor binds win).
- Mapping: `BindShortcuts` -> `shortcut-register` (upsert) + the user's
  chord via config/`shortcut-bind`; `ListShortcuts` -> `shortcuts` list;
  `ConfigureShortcuts` -> out-of-band config edit + reload (stage 1 has
  no in-compositor configure UI); `Activated` / `Deactivated` /
  `ShortcutsChanged` <- the `global-shortcut` subscription.
- Security: only the backend may register/receive in the stage-2 design
  (stage 1 exposes the primitives to any same-user IPC client; tighten
  here, e.g. by blessing one registrant identity).
- Live proof with a real portal client: a tiny test app or OBS on the M2,
  press the bound chord with another app focused, assert the portal
  signal arrives. Unit + headless integration first, same bar as stage 1
  (no per-event allocation on the input path, lifecycle cleanup).

## Not in this ticket

Stage 1 (landed). The deferred Wayland protocols (separate ticket):
`hyprland-global-shortcuts-v1`, `xx-hotkey-v1`.
