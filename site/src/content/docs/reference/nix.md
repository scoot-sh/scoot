---
title: Nix flake reference
description: "Every flake output scoot ships: packages, apps, modules, the overlay, and the macOS split."
---

The install pages show the commands. This page pins down what they
refer to.

## Outputs

Packages (`packages.<system>.*` on `x86_64-linux`):

| Name | Gives you | Pick it when |
|---|---|---|
| `scoot` | the compositor plus the `scoot msg` client (`$out/bin` carries only `scoot`) | the default everywhere; VM, container, nested-only |
| `scoot-gpu` | the same binary with the `gpu-scanout` feature | real hardware with a GPU (still named `scoot`) |
| `scoot-xwayland` / `scoot-gpu-xwayland` | those two with the `xwayland` feature and Xwayland on `PATH` (Linux only) | you run X11 apps |
| `scootbg` | the wallpaper daemon (Linux only) | `[wallpaper]` without the desktop profile |
| `scootbar` | the status bar, no font in its closure (Linux only) | the bar with your own fonts |
| `scootbar-demo` | the bar with a bundled font as its default | trying the bar on a box with no fonts |
| `default` | `scoot` on every system (client-only on macOS) | `nix run` without choosing |

Apps (`nix run` mirrors five of them — `default`, `scoot-gpu`,
`scoot-xwayland`, `scootbar` and `scootbar-demo`; no `scootbg`,
`scoot-gpu-xwayland` or docs-site app):

```sh
nix run github:scoot-sh/scoot -- --nested -- foot
nix run github:scoot-sh/scoot -- msg windows
nix run github:scoot-sh/scoot#scootbar-demo -- daemon --right clock
```

Modules: `nixosModules` and `homeModules` both ship `default`,
`scoot` and `scootbar`. The desktop profile's NixOS half is
`nixosModules.scoot` (`programs.scoot`, including `desktop`); its
user half is `homeModules.scoot` plus `homeModules.scootbar` for the
bar.

## The overlay

`overlays.default` adds `pkgs.scoot` and, on Linux, `pkgs.scootbg`
and `pkgs.scootbar`:

```nix
nixpkgs.overlays = [ inputs.scoot.overlays.default ];
```

They are the flake's own builds, the same derivations as
`packages.<system>.*`, not rebuilt against your nixpkgs — nothing is
built twice. With the overlay applied, the modules' `package` and
`wallpaper.package` default to these, which is what makes the pure
modules usable without the flake's wrappers. On macOS the overlay adds
the client-only `scoot` and no `scootbg` or `scootbar`. It never adds
`scootbar-demo`: a demo to run, not a package to build on.

Keep the flake's inputs following your nixpkgs, or the modules build
against a second nixpkgs:

```nix
home-manager.inputs.nixpkgs.follows = "nixpkgs";
```

Without the `follows`, the first rebuild compiles a duplicate
dependency tree.

## macOS: client only

On macOS, `scoot` is client-only: `scoot msg` drives a compositor
running on Linux. The home-manager module manages the config file on
any system (on macOS it also installs the client-only `scoot` — the
config you edit here deploys to a Linux box), while the NixOS module's
session entry only means anything on NixOS. `scootbg` and `scootbar`
are Linux-only: no macOS package, and on macOS a `[wallpaper]` section
renders as written and installs nothing.

## Symptoms

> **Symptom:** rebuild fails with `not of type 'TOML value'` naming
> `programs.scoot.settings`.
> A value in `settings` has no TOML representation (a Nix function).
> The option type-check fails at evaluation time, before anything
> builds.
> A value with the wrong scoot type (a string for `layout.gap`)
> still builds. scoot refuses it at session start.
> The loader fails safe — see [Failure
> semantics](../scoot/configure.md#failure-semantics).
