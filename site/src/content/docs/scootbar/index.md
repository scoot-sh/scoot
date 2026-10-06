---
title: scootbar overview
description: "The lightest status bar that is still beautiful — enable it, place modules, drive it."
---

A status bar for scoot, and for any compositor with
`wlr-layer-shell-v1`: the lightest bar that is still beautiful and
configurable. One thin strip, nothing knocked off the table. It starts as a clock, gains workspaces, and grows by
modules. It runs anywhere a layer-shell bar runs (scoot, sway, niri,
Hyprland) — on scoot it also answers the desktop profile's theming.

```sh
scootbar daemon --left workspaces --center clock
```

One binary, linking nothing beyond glibc and libgcc_s, and **no font
in its closure** — the font is yours to choose (even DejaVu Sans, the
first face the bar looks for, is 742 KiB against the bar's own ~826
KiB binary). It takes the font from `--font`, or the first of a short
list of well-known files; with neither it refuses to start, saying how
to give one. Trying it on a box with no fonts:

```sh
nix run github:scoot-sh/scoot#scootbar-demo                          # daemon, a clock
nix run github:scoot-sh/scoot#scootbar-demo -- daemon --right clock  # any daemon flags
```

(`scootbar-demo` is the same binary behind a script adding DejaVu
Sans — a demo, not what a system installs.)

## Enable it

With Nix, the modules are separate from the compositor's (importing
one changes nothing about the other):

```nix
imports = [ inputs.scoot.homeModules.scootbar ];   # or nixosModules.scootbar
programs.scootbar = {
  enable = true;
  features = [ "clock" "workspaces" ];   # optional: build with exactly these modules
  settings = {                           # bar.toml, any key
    left = [ "workspaces" ];
    center = [ "clock" ];
    bar.height = 32;
    colors.accent = "#89b4fa";
  };
};
```

| Option | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `enable` | bool | `false` | — | Installs the bar, writes its config and (unless `systemd.enable` is off) runs it. |
| `package` | package or null | the flake's `scootbar` | — | The bar. Null with `enable` is refused, by name. |
| `features` | list or null | `null` (defaults) | rebuild | The modules as Cargo features, **exactly** the ones listed (`[ ]` is a bar with no modules and no font). Null keeps `package` as it is. |
| `settings` | attrset | `{ }` | restart (unit) / `scootbar msg reload` by hand | Free-form: rendered as TOML to the bar's config, so a new option never needs a module change first. |
| `systemd.enable` | bool | `true` | — | The user service. Off: start `scootbar daemon` from your compositor's autostart instead (one route per program — both starts two daemons, and the second refuses). |

Without Nix: `scootbar daemon` runs in the foreground until the
compositor goes away (start it with `&`, or from `[autostart]`).
home-manager writes `~/.config/scoot/bar.toml` (the bar's default
path, so a hand-started daemon reads what the service does); NixOS
writes `/etc/scootbar/bar.toml`, named with `--config` in the system
unit.

Modules are Cargo features, reachable through `.override` — a bar
with no modules needs no font, and `icon-image` (the PNG decoder,
+115 KB) builds the same way (each line is one alternative):

```nix
scootbar.override { buildNoDefaultFeatures = true; }
```

```nix
scootbar.override { buildFeatures = [ "icon-image" ]; }
```

A bar built without a module refuses its table in the config (an
unknown key), naming it.

## Drive it

```sh
scootbar msg query                         # every placed module's state as JSON
scootbar msg layout                        # where each module is on screen, for a click
scootbar msg invoke volume raise 5         # run a module's action, as its click would
scootbar msg subscribe                     # stream changes, one JSON line each
scootbar msg reload                        # re-read the file and live-apply it
scootbar msg toggle                        # hide the bar (and release its space), or show it
```

Every module answers the pointer: clicks, scrolls and hover run a
module action, a command, or a request to scoot that the config binds.
A module can open a popup under itself (the volume slider, the network
list) and shows a tooltip after a hover delay. The config can define
its own `button`, a `push` target for `scootbar msg set`, and an
`exec` module streaming a command's output — with no Rust.

Next: [Configure](./configure.md) for layout and looks, [Modules](./modules.md) for the full set, [CLI reference](./cli.md) for flags and the agent interface.
