---
title: The scoot desktop
description: "One switch plus a look for the full lightweight desktop — bar, wallpaper, lock, notifications, hardware keys."
---
Turn on the profile, pick a look, rebuild, log out, and choose
**scoot** on your login screen. On NixOS that is two short additions,
one per side:

```nix title="configuration.nix"
programs.scoot = {
  enable = true;
  desktop.enable = true;
  look = "ginger-night";  # the login screen and system fonts follow it
};
```

```nix title="home.nix"
programs.scoot = {
  enable = true;
  desktop.enable = true;
  look = "ginger-night";  # or music-desk, radial-burst, moonrise, vinyl-sunset
};
```

Set the same look on both sides: your side themes the desktop, the
system side themes the login screen and picks the system fonts.

**You get** the bar, the wallpaper, idle and lock, notifications, a
launcher (`Super+d`), clipboard history, screenshots and screen
sharing, volume and brightness keys with an on-screen display, night
light, a polkit agent and keyring, and GTK and Qt themed to the look.
Power policy, the input method and the ReGreet login screen stay
opt-in.

![The full scoot desktop in the default look: the bar with an unread count, two terminals, the launcher mid-search and two notification cards over the cat-peeking wallpaper](../../../assets/full-look.png)

**Your other sessions and your login screen stay.** To undo, set
`desktop.enable = false;` on both sides and rebuild, or boot the
previous generation.

**Starting from nothing?** The three files (`flake.nix`,
`configuration.nix`, `home.nix`) are on [Install](../start/install.md#use-it-every-day),
ready to copy: one switch, one look, the whole desktop, rolled back together.

**Which file does a snippet go in?** Each one on these pages is titled
with its file. The system side (`configuration.nix`) installs services
and system packages; your side (`home.nix`) writes the config and runs
your user services. Most pieces need both halves: set them in both.

## Set up the flake

The [Install](../start/install.md#use-it-every-day) page has the whole
`flake.nix`. Prefer FlakeHub? The flake is published as `scoot-sh/scoot`
(once per merge to `main`, after both architectures' binaries reach
Cachix):

```nix title="flake.nix"
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    scoot.url = "https://flakehub.com/f/scoot-sh/scoot/*.tar.gz";
  };
}
```

or `fh add scoot-sh/scoot`. `*` follows the rolling release (every
merge to `main`): its numbers are one minor ahead of the last release
(`0.3.<commits>` while releases are 0.2.x), so they always sort newest.
A constraint like `0.3.*` stops following `main` once 0.3.0 is
released; pin a full version from the flake's page to hold still. Either way, set up the [binary cache](../start/binary-cache.md)
or Nix compiles Smithay and the crates locally.

Leave `scoot.inputs.nixpkgs.follows = "nixpkgs"` **off** unless you
have a reason: Cachix holds binaries built from scoot's own pinned
nixpkgs, so pointing it at yours makes the cache miss and the first
build compile locally. Which package name to take, the overlay,
`nix run` and the macOS split are in the [Nix flake
reference](../reference/nix.md).

## NixOS

The session entry comes on with the profile (without it, `session.enable`
stays an explicit opt-in — a login-screen change is a change to your way
back in, and that stays deliberate). The entry adds scoot *alongside*
your existing sessions, never replacing the default. The greeter replaces
the login screen, so it is opt-in twice over; it is refused at eval
beside GDM or SDDM. Logging in through it starts the same session [First
session](../start/first-session.md) describes.

### What a switch does

A switch never ends your session. The running compositor keeps running
from its old store path — even a switch that ships a new scoot build —
and the new build takes over at the next login. Session-bound units
whose own files changed (the bar, the notification daemon, the clipboard watchers) restart
against the still-running compositor; the display socket never moves,
so they reattach to the same session. If the compositor itself exits
instead — quitting, crashing, or being stopped — the session targets
stop with it, so nothing is left restart-looping on a missing display;
log back in for a fresh session on the new build. That includes
`systemctl --user restart scoot.service`: restarting the compositor ends
the session, so use it only when you mean to log out.

> **Symptom:** after a switch the bar logs "cannot connect to the
> Wayland compositor" over and over while the session targets stay
> active. That is a session outliving its compositor — the shape this
> unit setup replaced. Check which side went missing first
> (`systemctl --user status scoot.service` names the state; `journalctl
> --user -u scoot.service` names the exit), stop the leftovers
> (`systemctl --user stop scoot.service scoot-session.target`), and log
> in again.

Which options live on which side:

| Side | Owns |
|---|---|
| NixOS (`programs.scoot` in `configuration.nix`) | the package system-wide; the login-screen session entry; system-wide scootbg (so `[wallpaper]` finds it on `PATH`); the idle/lock tools system-wide, the docked-lid rule, and the locker's PAM service; the portal backends system-wide plus PipeWire running; the OSD installed system-wide; the night-light tool system-wide; the polkit authority, the agent and the keyring system-wide plus the greetd keyring-unlock PAM pair; system dconf (for the theme's dark-mode signal); the profiles daemon, the lid/power-key/low-battery policy and the charge service (all opt-in through `power.enable`); the greeter |
| Home Manager (`programs.scoot` in `home.nix`) | the themed config file; the portal config; the per-desktop chooser config xdpw asks through; the screenshot tools; the secrets client; the user units (idle policy, notification daemon, clipboard watchers, night light, bar feed, OSD, polkit agent) plus the `scoot-session.target` scope they start in; the volume/brightness/sink scripts beside the keymap's binds; the profile switch, the charge button's fill unit, and the tools for the user |

## Home Manager

Standalone home-manager (Nix on another distro, or a `home.nix` kept next
to the machine that edits it, which also manages the config on macOS for
the Linux box it deploys to) takes the same `home.nix`.
The flake for it is on [Install](../start/install.md#use-it-every-day),
under **Nix on another Linux**.

A home-manager-only setup still needs two things from wherever PAM
and the seat are configured: the locker's PAM service (else no
password unlocks it), and backlight rights for dimming (else the dim
step logs EPERM and does nothing).

## Which sessions start the units

Every profile unit — the idle pair, the notification daemon and its bar feed, both
clipboard watchers, and the profile-managed bar — starts in
`scoot-session.target`, scoot's own session scope, and stops when it
ends. That scope is what the launcher starts past the display import,
so the display is already in the user manager when the units start.
No other desktop starts them: logging in to GNOME, KDE, niri or
Hyprland with the same home-manager config leaves scoot's notification
daemon,
locker policy and clipboard history stopped, instead of running
them inside someone else's session. (A standalone bar — the bar
module without the profile — stays a generic
`graphical-session.target` unit.)

Without the launcher — a hand-written greetd entry or `startwm.sh`
that runs `scoot --tty` directly instead of `scoot-session` — start
the scope by hand once the display is known, and stop it on the way
out:

```sh
systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE
systemctl --user start scoot-session.target
# ... on exit:
systemctl --user stop scoot-session.target
```

The home-manager side installs the scope's target file itself, so
this works with no NixOS login entry. Units ordered after the scope
inherit the imported display from the user manager. A
`sessionScript` entry (`scoot --tty -- session.sh`) gets none of this
wiring — for startup programs, use `[autostart]`
instead.

## Without flakes

`nix run github:scoot-sh/scoot -- --nested -- foot` tries it with
nothing installed, and `nix profile add` keeps it (see
[Install](../start/install.md)). Not on NixOS at all? The same page's build-from-source
route plus your own bar, launcher and session script — the profile is
optional, and everything it automates is documented
across this site.

Which build sits underneath is still your call: the profile never sets
`programs.scoot.package`, so [pick a build](../start/install.md#which-build-do-i-need)
there (the default; X11 is a runtime knob, not a package) and the whole desktop follows it.

Two loud refusals instead of silent no-ops: `desktop.enable` without
`programs.scoot.enable`, and a `look` without `desktop.enable`, each fail
evaluation naming the missing switch; an unknown `look` fails naming the
catalog.

## The pieces

Each piece of the profile has its own page.

## Pick a look

Choose the look, what the app theme covers, and a wallpaper from a link. See [Look and theme](./look.md).

## App theme

Moved to [Look and theme](./look.md#app-theme).

## Wallpaper from a link

Moved to [Look and theme](./look.md#wallpaper-from-a-link).

## Idle and lock

The dim, lock and screen-off schedule, the locker and its PAM setup. See [Idle and lock](./idle-lock.md).

## Notifications

scootnotify as the notification daemon, and moving over from mako. See [Notifications](./notifications.md).

<a id="migrating-from-mako"></a>

## Clipboard

A history that outlives the app that copied, with a picker. See [Clipboard](./clipboard.md).

## Launcher

The `Super+d` launcher, and moving over from fuzzel. See [Launcher](./launcher.md).

<a id="migrating-from-fuzzel"></a>

## Screenshots and screen sharing

Screen sharing through the portals, and screenshots. See [Screenshots and screen sharing](./screenshots.md).

## Sound, brightness keys and the on-screen display

PipeWire, the volume and brightness keys, and the on-screen display. See [Sound, brightness keys and the on-screen display](./sound.md).

## Night light

Warm the screen after dark. See [Night light](./night-light.md).

## Privilege prompts and the keyring

The polkit agent and the keyring. See [Privilege prompts and the keyring](./auth.md).

## Terminal, files, and removable media

The terminal, file opening, user directories and automounting. See [Terminal, files, and removable media](./apps.md).

## WiFi and Bluetooth

The WiFi and Bluetooth pickers. See [WiFi and Bluetooth](./network.md).

## Hardware keys and desktop actions

The keymap for hardware keys and desktop actions. See [Hardware keys and desktop actions](./keys.md).

## Power

Power profiles, lid and low-battery policy, and a charge limit (opt-in). See [Power](./power.md).

<a id="profiles"></a>

<a id="lid-power-key-low-battery"></a>

<a id="charge-limit"></a>

## Displays

Scale and power that follow the connected monitors. See [Displays](./displays.md).

## The greeter

The greeter, and a TTY autologin without one. See [The login screen](./login.md).

## Without a greeter (TTY autologin)

Moved to [The login screen](./login.md#without-a-greeter-tty-autologin).

## Input method

fcitx5 for CJK and compose input (opt-in). See [Input method](./input-method.md).

<a id="trying-it-headless"></a>

## XWayland, and what comes next

**XWayland** is a runtime knob on the single package: the profile's
`desktop.xwayland.enable` defaults the compositor's `[xwayland] enabled`
on, and the default `scoot` package already carries XWayland — leave
`programs.scoot.package` at the default and turn the knob on as in
[the install page](../start/install.md#which-build-do-i-need).

**Every later piece has its slot already**, off and inert: one boolean
(plus a package override where a package is involved) per paved-path
child, so those children fill bodies without renaming options:

| Slot | Type | Default | Child |
|---|---|---|---|
| `desktop.launcher.enable` (+ `daemon`) | bool (+ enum, package) | `true` ([Launcher](./launcher.md): scootlaunch on `Super+d` and `Ctrl+Alt+Space`, overlay layer, themed, nothing held when closed) | launcher (scootlaunch; fuzzel one line away) |
| `desktop.capture.enable` | bool + packages | `true` ([Screenshots and screen sharing](./screenshots.md): portal backends, PipeWire, grim + slurp, the output chooser) |
| `desktop.auth.enable` / `desktop.secrets.enable` | bool + packages | `true` ([Privilege prompts and the keyring](./auth.md): polkit-gnome agent spawned by the session leader, gnome-keyring D-Bus activated with one unlock per login) | polkit agent + keyring |
| `desktop.audio.enable` | bool + package | `true` ([Sound, brightness keys and the on-screen display](./sound.md): PipeWire with WirePlumber, the OSD on `overlay`, the binds through its scripts) |
| `desktop.clipboard.enable` (+ `maxItems`, `dbPath`, three packages) | bool (+ int, path, packages) | `true` ([Clipboard](./clipboard.md): history kept, picker bound, wiped at lock) | clipboard persistence + history (lean cliphist + wl-clipboard + the launcher's menu) |
| `desktop.nightlight.enable` | bool + package | `true` ([Night light](./night-light.md): wlsunset on the manual schedule, gammastep for location mode, themed by the look) | night light (wlsunset now, gammastep beside it; a future scoot-native keeps the names) |
| `desktop.power.enable` | bool + package | `false` (opt-in, never with the profile — [Power](./power.md): profiles on `Super+p`, lid/low-battery suspend, charge limit) | power profiles, suspend, charge limit |
| `desktop.theme.enable` | bool + package | `false` | GTK/Qt theme, dark mode |
| `desktop.apps.terminal.enable` / `desktop.apps.fileManager.enable` | bool + package | terminal `true`, file manager `false` ([Terminal, files, and removable media](./apps.md): foot, `xdg-open` and the user dirs; pcmanfm opt-in) | terminal + file manager |
| `desktop.apps.network.enable` / `desktop.apps.bluetooth.enable` | bool + packages | `true` ([WiFi and Bluetooth](./network.md): pickers on `Super+w` / `Super+b` and the bar's modules) | network + Bluetooth pickers |
| `desktop.displays.enable` (+ `profiles`) | bool (+ list) | `true` ([Displays](./displays.md): the watcher re-matches the connected set at login, on every plug and unplug, and on idle resume, sets scale and power live over IPC, never writing the config, and turns back on only the outputs it turned off) | output policy (scoot-native profiles, a stopgap: stock kanshi cannot drive the read-only output-management write half) |
| `desktop.inputMethod.enable` (+ `daemon`, `addons`, package) | bool (+ enum, list, package) | `false` (opt-in, never with the profile — [Input method](./input-method.md): fcitx5 as a user unit, toolkit variables where needed, candidate window themed) | input-method engine (fcitx5 now; a future engine keeps the names) |
| `desktop.automount.enable` | bool + package | `true` ([Terminal, files, and removable media](./apps.md): udiskie trayless over udisks2, internal disks ignored) | removable-media automount |

Every slot above has its body: `false` means its daemon is not
started, not that the option is reserved. Changes apply on
rebuild/switch.
