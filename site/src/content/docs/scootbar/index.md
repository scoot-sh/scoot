---
title: scootbar overview
description: "The lightest status bar that is still beautiful — enable it, place modules, drive it."
---

A status bar for scoot and for any compositor with
`wlr-layer-shell-v1` (scoot, sway, niri, Hyprland). On scoot it follows
the desktop profile's look. It starts as a clock and grows by modules.

```sh
scootbar daemon --left workspaces --center clock
```

One binary, linking nothing beyond glibc and libgcc_s, with **no font
in its closure**: you choose the font. It takes the font from `--font`, or the first of a short
list of well-known files. With neither it refuses to start and says how
to give one. To try it on a box with no fonts:

```sh
nix run github:scoot-sh/scoot#scootbar-demo                          # daemon, a clock
nix run github:scoot-sh/scoot#scootbar-demo -- daemon --right clock  # any daemon flags
```

(`scootbar-demo` is the same binary behind a script that adds DejaVu
Sans.)

## Enable it

With Nix, the bar's module is separate from the compositor's. Importing
one changes nothing about the other:

```nix title="home.nix"
imports = [ inputs.scoot.homeModules.scootbar ];   # skip if the Install flake already loads it
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

The snippets below use `inputs` and `pkgs`, so `home.nix` needs
`{ inputs, pkgs, ... }:` at the top. Both
[Install](../start/install/) flakes pass those down; a bare
`{ ... }` file has neither in scope.

Without Nix, `scootbar daemon` runs in the foreground until the
compositor goes away. Start it with `&`, or from `[autostart]`.
home-manager writes `~/.config/scoot/bar.toml` (the bar's default
path, so a hand-started daemon reads what the service does); NixOS
writes `/etc/scootbar/bar.toml`, named with `--config` in the system
unit.

scoot's own config has no `[bar]` section. The
service above (or `[autostart]` without Nix) is the only way to
start the bar. A config change restarts it through the service
(`scootbar msg reload` by hand).

### When the bar exits

The bar never reconnects. When the compositor goes away it exits with
status 1 (see [Exit status](./cli.md#exit-status)); so does a crash.
The Nix units set `Restart=on-failure` and `RestartSec=2` with no start
limit, so either is followed by a fresh start two seconds later, and it
retries until the compositor is back. `scootbar msg kill` and a plain
stop exit cleanly and stay stopped.

Without the unit nothing restarts it: after a compositor restart there is
no bar until your autostart, or you, run `scootbar daemon` again.
scoot does not supervise its clients.

Modules are Cargo features, reachable through `.override`. A bar
with no modules needs no font, and `icon-image` (the PNG decoder,
+115 KB) builds the same way. Each block is one alternative:

```nix title="home.nix"
programs.scootbar.package = inputs.scoot.packages.${pkgs.stdenv.hostPlatform.system}.scootbar.override { buildNoDefaultFeatures = true; };
```

```nix title="home.nix"
programs.scootbar.package = inputs.scoot.packages.${pkgs.stdenv.hostPlatform.system}.scootbar.override { buildFeatures = [ "icon-image" ]; };
```

A bar built without a module refuses its table in the config as an
unknown key, and names it.

## Drive it

```sh
scootbar msg query                         # every placed module's state as JSON
scootbar msg layout                        # where each module is on screen, for a click
scootbar msg invoke volume set 50          # run a module's action, as its click would
scootbar msg subscribe                     # stream changes, one JSON line each
scootbar msg reload                        # re-read the file and live-apply it
scootbar msg toggle                        # hide the bar (and release its space), or show it
```

Every module answers the pointer: clicks, scrolls and hover run a
module action, a command, or a request to scoot that the config binds.
A module can open a popup under itself (the volume slider, the network
list) and shows a tooltip after a hover delay. The config can also define
its own `button`, `push`, and `exec` modules, with no Rust.

Next: [Configure](./configure.md) for layout and looks, [Modules](./modules.md) for the full set, [CLI reference](./cli.md) for flags and the agent interface.
