---
title: scootlaunch overview
description: "The keyboard-first application launcher — the desktop entry index, ranked search, the overlay panel, launching, and the dmenu and run modes."
---

A keyboard-first application launcher for scoot and any compositor
with `wlr-layer-shell-v1`. What ships today is the desktop entry
index, ranked search, the headless agent interface, the overlay panel
with its entry and virtualized list, launching, and the dmenu and run
modes.

```sh
scootlaunch --list --json                  # every visible entry, as JSON
scootlaunch --query "fire" --json          # ranked as the UI would rank it
scootlaunch                                # open the panel
printf 'a\nb\n' | scootlaunch --dmenu      # pick a line, print it
scootlaunch --run                          # the panel over $PATH executables
```

One binary, holding nothing when closed. It reads the
same desktop files every launcher reads (`$XDG_DATA_HOME` then each
`$XDG_DATA_DIRS` entry's `applications/`, subdirectories walked,
first file per id wins, `Hidden=true` masking across data dirs),
expands each `Exec` to an argv per the Desktop Entry Specification,
and caches the result under `$XDG_CACHE_HOME/scootlaunch/index-v1`.

## What it does today

- Lists indexed entries, with `NoDisplay` and
  `OnlyShowIn`/`NotShowIn`/`TryExec`-hidden entries reachable through
  `--all` with their reason (`--list --all --json`).
- Ranks entries for a query (`--query TEXT`): a case-folded fuzzy
  match over the name, generic name, keywords and Exec basename,
  plus a frecency bonus from `$XDG_STATE_HOME/scootlaunch/history`.
  The bonus never outranks a much better text match. An empty query
  orders by frecency, then name.
- Prints the agent JSON schema: `id`, `name`, `generic_name`,
  `comment`, `keywords`, `icon`, `exec` (the expanded argv),
  `terminal`, `dbus_activatable`, `actions`, `path` and `hidden`.
  `--query --json` prints the same schema in ranked order.
- Expands `Exec` strictly per the spec, or with `--lenient` the way
  GLib reads the real files that break the quoting rule.
- Opens the panel (bare `scootlaunch`): the overlay
  exclusive-keyboard surface with the entry and the ranked,
  match-highlighted list. Details, keys and theming are in [The
  panel](./panel.md). Picking launches the entry and exits 0 (a dmenu
  pick prints instead of launching; see the [CLI
  reference](./cli.md)).

Every flag is in the [CLI reference](./cli.md). An app missing from
the list is diagnosed there too.

## Nix

Two modules, one switch. `programs.scootlaunch` manages the
launcher's own file; `programs.scoot.desktop.launcher.daemon` picks
the menu behind the keymap's binds. The look reaches the panel as a
runtime layer: a value you set still wins per key. Stylix reaches it
only as the `stylix` look — there is no Stylix switch and no unit
here (nothing runs when the panel is closed).

| Option | Type / default | Example | Reload |
|---|---|---|---|
| `programs.scootlaunch.enable` | bool, `false` (`true` with the profile opt-in below) | `true` | rebuild or switch |
| `programs.scootlaunch.package` | package or null, the flake's build | `inputs.scoot.packages.${pkgs.stdenv.hostPlatform.system}.scootlaunch` | rebuild or switch |
| `programs.scootlaunch.features` | list of strings or null, `null` (default features: SVG icons on) | `[ ]` builds without SVG icons | rebuild or switch |
| `programs.scootlaunch.settings` | TOML table, `{ }` | `{ width_em = 50; rows = 12; }` | next open (the file is read whole at every open) |

The file lands at `scoot/launch.toml` under `$XDG_CONFIG_HOME`
(home-manager) or `/etc/scootlaunch/launch.toml` (NixOS) and holds
only these settings, verbatim. The keys are the launcher's own
(`font`, `width_em`, `rows`); anything else is refused by name —
`--check` says which. An empty file is the built-in defaults.

The opt-in, beside the profile:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.launcher.daemon = "scootlaunch";
```

With it, `scoot-launcher` execs `scootlaunch` with no color flags,
and the clipboard picker, the WiFi/Bluetooth pickers and the
screencast chooser run it through `--dmenu` the same way. To manage
the file beside the switch, also import the module
(`scoot.homeModules.scootlaunch`, `scoot.nixosModules.scootlaunch`
on the system side): the daemon switch alone runs the flake's build
with no file. The keys, the panel and the look layer are in [The
panel](./panel.md).

To go back, drop the line (or set `"fuzzel"`) and switch again. The
default stays `"fuzzel"`: nothing changes unless the line names
`scootlaunch`.

> **Symptom:** the menu ignores your `fuzzel.ini` font settings.
>
> `scootlaunch` never reads `fuzzel.ini`: set the face in
> `programs.scootlaunch.settings.font`, or switch the daemon back to
> `"fuzzel"`.

> **Symptom:** `Super+d` opens nothing after the switch.
>
> The bind renders only with the launcher slot on (it is, with the
> profile). A `daemon` typo fails the evaluation naming the value —
> rebuild says which.
