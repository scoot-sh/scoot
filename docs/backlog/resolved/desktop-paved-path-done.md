---
title: "Paved path to a full lightweight desktop from the flake (epic)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Paved path to a full lightweight desktop from the flake (epic)

Filed 2026-10-04 from the maintainer brief: "Our flake should have the
paved path for essentially a full lightweight DE." Serves **daily-drive**
first (a user gets a working desktop from one enable plus a look choice,
instead of hand-wiring what the maintainer hand-wired on their Asahi M2:
swayidle dim 2 min / `wlopm --off` 5 min, a charge-limit service, a
speakers-heal unit) — and **computer use** second (every slot wired means an
agent's VM session runs the same daemons a human's does: notifications,
launcher, clipboard, screenshots).

## Inventory

Verified 2026-10-04 against `origin/main` (`73fdb4f25`), from
`nix/modules/*.nix`, `flake.nix`, `docs/nix.md`, `docs/configuration.md`,
`docs/protocols.md`, `nix/tests.nix`, and the backlogs.

| # | Piece | Status | Evidence |
|---|---|---|---|
| 1 | Session + greeter | **provided** | `nix/modules/nixos.nix`: session entry `sessionPackage` (38-56), `session.*` (162-235) and `greeter.*` (237-279) options; `docs/nix.md` "NixOS module" + "What a greeter login starts" + "The greeter: ReGreet, opt-in" |
| 2 | Lock screen | **missing** | `ext-session-lock-v1` implemented (`docs/protocols.md:27`); no locker packaged or wired anywhere in `nix/` — no locker is named in `nix/` (`docs/protocols.md:1903` lists `swaylock`, `gtklock`, `hyprlock` and `waylock` as working); the config docs' only example is `swaylock` inside a hand-written snippet (`docs/configuration.md:826`) |
| 3 | Idle policy (dim, lock, screens off, lock before sleep, media inhibit) | **partly** | `ext-idle-notify-v1` v2 + `idle-inhibit-v1` (`protocols.md:28-29`), `wlr-output-power-management-v1` (`protocols.md:30`, #427); policy is a hand-written swayidle script (`configuration.md:816-847`); no dim action, no lock-before-sleep, no inhibit-while-media wiring |
| 4 | Notifications | **missing** | `scootnotify` is a pointer only (`docs/scootbar/backlog/scootnotify.md`: low, M7, blocked on the maintainer starting it); `mako` appears only in examples (`nix/modules/home.nix:210`, `configuration.md:774`) |
| 5 | Launcher | **missing** | `scootlaunch` is a pointer only (`docs/scootbar/backlog/launcher.md`: low, M7); the default binds reference `wofi` (`docs/nix.md:308`) but the flake installs no launcher |
| 6 | xdg-desktop-portal backends (screenshot/screencast/file chooser) | **partly** | `portals.enable` + `resources/scoot-portals.conf` (ScreenCast/Screenshot to `wlr`, needs xdpw 0.8.0+, avoid 0.8.3; rest to `gtk`); no module installs `xdg-desktop-portal`, `-wlr`, `-gtk`, or `grim` (>= 1.5.0, required: `scoot-portals.conf:33-36`) |
| 7 | Polkit agent | **missing** | zero mentions in `nix/`, `flake.nix`, `docs/nix.md` |
| 8 | Secrets/keyring | **missing** | zero mentions in `nix/`, `flake.nix`, `docs/nix.md` |
| 9 | Audio/brightness/media keys + OSD | **partly** | scootbar already ships `volume`, `brightness`, `media`, `microphone` modules (`crates/scootbar/src/modules/`); no default key binds, no OSD, no pipewire/wireplumber baseline in the flake |
| 10 | Screenshots bound to keys | **partly** | `grim` works via `ext-image-copy-capture-v1`, `scoot msg screenshot` exists; no binds, `grim`/`slurp` installed by nothing |
| 11 | Clipboard persistence + history | **missing** | compositor side complete: `wlr-data-control-v1` v2, `ext-data-control-v1` v1, focus-gated `primary-selection-v1` (`protocols.md:31-33, 2079-2093`), XWayland crossing; no manager, no `wl-copy`/`wl-paste` on PATH, no picker bind. Required slot per maintainer addendum 2026-10-04 |
| 12 | Night light | **partly** | `wlr-gamma-control-v1` implemented (`protocols.md:34, 2095-2120`, names `gammastep`/`wlsunset` as working, incl. `--tty` LUT behavior); nothing wired in the flake |
| 13 | Power (profiles, lid/low-battery suspend, charge limit) | **missing** | scootbar has `battery`/`power` *display* modules only; no `power-profiles-daemon`, no logind wiring, no charge-limit option (the M2's is hand-wired local config) |
| 14 | Fonts, cursor, GTK/Qt theme, dark mode from the look | **partly** | Stylix wiring for scoot + bar exists (`nix/modules/home.nix:136-172`, `docs/nix.md:362-397`); cursor via Stylix; no GTK/Qt settings, no dark-mode signal, no non-Stylix fallback; the three example looks hand-write per-app configs |
| 15 | Terminal | **partly** | `super+Return` spawns `foot` by default (live defaults in `docs/nix.md`); examples ship `foot.ini`; the flake installs no terminal |
| 16 | File manager | **missing (optional)** | nothing referenced anywhere; explicitly optional |
| 17 | Network/Bluetooth UI | **partly** | scootbar has `network`/`bluetooth` modules, but `bluetooth-real-hardware.md` is open and `network-child-stuck` is claimed/in-flight; no pickers |
| 18 | Removable-media automount | **missing** | `udiskie` zero mentions in `nix/`, `docs/` |
| 19 | XWayland | **provided** | `scoot-xwayland` / `scoot-gpu-xwayland` packages + `[xwayland] enabled` knob (`docs/nix.md:233-284`) |

Already-provided pieces the profile reuses without change: session +
greeter (#1), wallpaper/scootbg incl. Stylix (`docs/nix.md:702-764`),
scootbar (`docs/nix.md:766-1005`), portal *config* (#6, config half),
XWayland (#19), and the three example looks in `docs/examples/*` as the
first `look` values.

## The design (what each child implements against)

**What the user writes:**

```nix
programs.scoot.desktop = {
  enable = true;
  look = "vinyl-sunset";   # one of docs/examples/*, default null (no theming)
};
# Every piece individually overridable / disable-able:
# programs.scoot.desktop.idle.lock.enable = false;
# programs.scoot.desktop.notifications.daemon = "mako";  # default
```

- `programs.scoot.desktop.enable = true` plus an optional
  `look = "<example-look>"` that themes every piece (compositor
  `[appearance]`, bar `colors`, foot palette, GTK/Qt/dark mode, greeter
  CSS pick, wallpaper) — the same palette, not five hand-synced files.
- **NixOS vs Home Manager split:** the NixOS module owns system services,
  backend packages (portals, polkit, power, automount) and the greeter;
  the home-manager module owns user units, binds, theme files and per-user
  config. `desktop.enable` exists on both sides; either side alone degrades
  to what it can do (as the `scoot`/`scootbar` modules already do).
- **Every piece individually overridable/disable-able:** one boolean per
  slot (e.g. `idle.lock.enable`, `notifications.enable`), package overrides
  beside each (as `wallpaper.package` / `features` already do), user values
  winning over look defaults per key (the Stylix `mkDefault` precedence
  pattern, pinned in `nix/tests.nix`).
- **Lightweight defaults** (pick the lightest well-maintained tool per slot,
  say why in the child; measure closure size): locker over
  `ext-session-lock-v1` (candidates: swaylock/waylock/gtklock/hyprlock —
  all work per the protocol table); idle `swayidle` (the docs already
  standardize on it); notifications `mako`; launcher `fuzzel`
  (layer-shell `overlay` native, no toolkit; note the default binds name
  `wofi` today — reconcile); screenshots `grim`+`slurp`; clipboard
  `cliphist`+`wl-clipboard`; night light `wlsunset` (single purpose) vs
  `gammastep`; automount `udiskie`; terminal `foot` (already the default
  bind). Prefer standard protocols throughout per CLAUDE.md.
- **Native-replacement contract:** a scoot-native piece later
  (`scootnotify`, `scootlaunch`, a future scootlock) replaces a slot
  *without changing the user's config* — same option names, same binds,
  daemon name as the only visible change. Children reference (not
  duplicate) the `scootnotify`/`scootlaunch` pointer entries.
- **What stays out:** no network-manager takeover, no display-manager
  replacement beyond the existing opt-in greeter, nothing that can strand a
  login (never set `defaultSession`, never force autologin, every
  login-screen change stays opt-in per the never-strand rule). No firewall,
  no distro packaging.

## Children and suggested order

Suggested order (not `blocked`-links — each must stay pickable on its own;
only real blockers are recorded as `blocked`):

1. `desktop-profile` — the `desktop.enable` + `look` option shell, wiring
   only what exists today (#1, wallpaper, bar, portal config, XWayland).
2. `desktop-idle-lock` — idle policy + locker (highest value; the M2's
   hand-wired swayidle is the reference). Blocked on
   `fix/scoot-session-target-after-display`.
3. `desktop-notifications` — mako now, scootnotify later. Blocked on the
   same branch (user unit would start before `WAYLAND_DISPLAY` import).
4. `desktop-launcher`, 5. `desktop-capture`, 6. `desktop-auth-secrets`,
   7. `desktop-audio-osd`, 8. `desktop-clipboard` (required slot,
   addendum 2026-10-04), 9. `desktop-nightlight`, 10. `desktop-power`,
   11. `desktop-theme-look`, 12. `desktop-apps`, 13. `desktop-keys` (one keymap every child
   registers into: hardware keys and desktop actions; high, filed after
   the maintainer's "default key shortcuts" ask), 14. `desktop-displays`,
   15. `desktop-input-method` (off by default) — each extends the profile;
   each carries eval pins in `nix/tests.nix`, a real-login proof on the M2
   where it matters (lock, idle, power, greeter-adjacent), and docs in
   `docs/nix.md`.

Known blocker (brief direction): the session target is reached before
`WAYLAND_DISPLAY` is imported — being fixed on branch
`fix/scoot-session-target-after-display` (exists at this head, no commits
above `origin/main` yet). Idle/lock and notification user units would start
too early and skip, so those two children record it as `blocked`.

## Not in this epic

Implementation (this epic files the plan only); compositor-side protocol
work (all needed protocols exist); distro packages (see the
`arch/deb/rpm-package` entries); the `scootnotify`/`scootlaunch` builds
themselves (pointer entries in `docs/scootbar/backlog/`).

## Resolution (2026-10-07)

Fourteen of fifteen children resolved, each in its own merged PR; every one
of the 19 inventory rows is shipped (row 16 was explicitly optional and is
covered opt-in). The remainder is exactly one open entry,
`desktop-input-method` (low, off by default; a reserved `configSlot` plus a
docs row already exist for it), which stays open on its own.

Inventory rows to merged PRs (all commits on `origin/main`, verified by
`git log` 2026-10-07; modules in `nix/modules/`, user docs in
`site/src/content/docs/desktop/index.md`, eval pins in `nix/tests.nix`):

| # | Piece | Shipped in |
|---|---|---|
| 1 | Session + greeter | provided pre-epic (`nix/modules/nixos.nix` session/greeter options) |
| 2 | Lock screen | PR #435 `e11400785` (swaylock over `ext-session-lock-v1`, `idle.lock.daemon`) |
| 3 | Idle policy | PR #435 (swayidle dim/lock/off, media inhibit, lock-before-sleep) |
| 4 | Notifications | PR #441 `3293ad680` (mako now, scootnotify later; bar feed) |
| 5 | Launcher | PR #451 `aee00c434` (fuzzel now, scootlaunch later) |
| 6 | Portals + capture | PR #459 `4409cfb14` (portal backends, grim/slurp, screenshot keys) |
| 7 | Polkit agent | PR #477 `da790e8e2` (polkit-gnome) |
| 8 | Secrets/keyring | PR #477 (gnome-keyring, D-Bus activated) |
| 9 | Audio/brightness/media keys + OSD | PR #471 `893957bba` (PipeWire baseline, keymap binds, OSD) |
| 10 | Screenshots on keys | PR #459 (grim/slurp binds) |
| 11 | Clipboard persistence + history | PR #443 `98cfc442f` (cliphist, picker, lock policy) |
| 12 | Night light | PR #472 `9d0c65eae` (wlsunset user unit) |
| 13 | Power | PR #465 `476cda70a` (power-profiles-daemon, logind policy, charge limit) |
| 14 | Fonts/cursor/GTK/Qt/dark mode from the look | PR #474 `d65e62c7e` (per-target opt-outs, Stylix precedence) |
| 15 | Terminal | PR #484 `ae456f7e0` (foot installed + themed) |
| 16 | File manager (optional) | PR #484 (opt-in pcmanfm) |
| 17 | Network/Bluetooth UI | PR #484 (keyboard WiFi/Bluetooth pickers) |
| 18 | Automount | PR #484 (trayless udiskie over udisks2) |
| 19 | XWayland | provided pre-epic (`scoot-xwayland` packages, `[xwayland] enabled`) |

Profile shell, keymap and displays (the design's cross-cutting pieces):

| Child | Resolved | Landed in |
|---|---|---|
| `desktop-profile` | 2026-10-04 | PR #432 `85cbe326a` (`desktop.enable` + `look`, future slots) |
| `desktop-keys` | 2026-10-05 | PR #442 `26e6c48f7` (one default keymap) |
| `desktop-displays` | 2026-10-06 | PR #482 `622169331` (scoot-native display profiles over IPC) |

Plus `desktop-idle-lock` 2026-10-04, `desktop-notifications` 2026-10-05,
`desktop-launcher` 2026-10-05, `desktop-capture` 2026-10-05,
`desktop-clipboard` 2026-10-05, `desktop-power` 2026-10-05,
`desktop-audio-osd` 2026-10-06, `desktop-nightlight` 2026-10-06,
`desktop-theme-look` 2026-10-06, `desktop-auth-secrets` 2026-10-06,
`desktop-apps` 2026-10-06 (each file in `docs/backlog/resolved/` names its
PR). The known `WAYLAND_DISPLAY`-import blocker from the brief is gone:
`fix/nix` `380cf0ba6` binds the profile units to `scoot-session.target`.
