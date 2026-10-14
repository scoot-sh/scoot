---
title: Builds and renderers
description: "The one scoot package, how the renderer is chosen, and the CPU-only minimal build."
---

There is one build: `scoot`. It runs everywhere — on real hardware it
scans out from the GPU, in a VM or container it renders on the CPU —
and its default (`auto`) picks the right renderer per session. This page
is about how that choice works, how to override it, and the one
special case: X11 apps and the CPU-only minimal build.

## Which build do I need?

Just `scoot`, from the binary cache
([Use the binary cache](../start/binary-cache.md)). The GPU scanout
tier is compiled in and reaches the GPU libraries at run time, so the
same binary starts with and without a GPU stack installed. What used to
be a build choice is now a run-time choice
([Which renderer draws the frames](#which-renderer-draws-the-frames)).

The only remaining build axis is **whether you run X11 apps**:

| Package | Draws with | Needs | Pick it when |
|---|---|---|---|
| `scoot` (default) | CPU or GPU, picked at run time | nothing | everyone, unless the next row |
| `scoot-xwayland` | as `scoot` | as `scoot`, plus X11 apps to run | you run X11 apps |
| `scoot-minimal` | the CPU renderer only | nothing | you want no GPU code at all (below) |

The `-xwayland` package carries a ~344 MiB larger closure (all but
~12 MiB of it Xwayland's own, which links Mesa) and a trust decision —
any X client can read other X clients' keys by design. Take it only if
you run X11 apps. (More in [XWayland](../scoot/xwayland.md).)

One visible side effect of the `-xwayland` package: its wrapper
leaves the process name as `.scoot-wrapped`, not `scoot` — so `pgrep -x
scoot` finds nothing there; use `pgrep -x .scoot-wrapped`.

### Do I have a GPU?

You do not need to answer this to pick a build anymore — but `auto`
behaves differently with and without one, so it is still worth one
command:

```sh
ls /dev/dri/renderD*
```

- **No output (no such directory, no matches)** → no GPU to use.
  The VM and container case is first-class, not degraded: `auto`
  resolves to the CPU renderer there, and running with no GPU is a hard
  requirement, not a fallback tier.
- **A render node listed (for example `renderD128`)** → a GPU exists.
  On real hardware, `auto` picks the GPU renderer under `--tty`. On a
  VM the node is usually software (llvmpipe) — there the GPU renderer
  runs several times *slower* than the CPU renderer, so `auto` stays on
  the CPU renderer unless you measured otherwise (pass `--renderer gpu`
  explicitly to override it).

### Say it in the config

The package knob is `programs.scoot.package` (home-manager and NixOS
alike; the desktop profile never sets it — the choice stays yours).
Leave the default (omit `package`) unless you run X11 apps:

```nix title="configuration.nix"
programs.scoot = {
  enable = true;
  # ...plus X11 apps? Take the XWayland build.
  # package = inputs.scoot.packages.${pkgs.stdenv.hostPlatform.system}.scoot-xwayland;
};
```

`inputs` and `pkgs` need `{ inputs, pkgs, ... }:` at the top of
`configuration.nix` (the Install page's `flake.nix` passes `inputs`
down through `specialArgs`).

The same names work with `nix profile add` and `nix run` — swap `scoot`
for `scoot-xwayland` (or `scoot-minimal`) wherever it appears.

## Which renderer draws the frames

`--renderer cpu|gpu|auto` (config: `[renderer] backend`, env:
`SCOOT_RENDERER`) picks what composites each frame. The default is
`auto`: the GPU renderer on real hardware under `--tty`, the CPU
renderer in VMs, containers, nested and headless sessions. An
explicit `--renderer cpu` or `--renderer gpu` always wins over `auto`.

Every session logs one line saying what was decided and why —
`requested=… source=… tier=… reason=…` with `renderer chosen` on it.
When the tier is not what you expected, read `reason=` there first.
Full rules, per-session table and symptoms:
[Backends and rendering](../scoot/backends.md#which-renderer-draws-the-frames).

`--renderer gpu` on a box with no GPU stack does not silently degrade:
the session warns naming what was missing and keeps the CPU renderer
with dumb buffers. `auto` on the same box resolves to the CPU renderer
with the reason in the log line — it never fails startup.

On real hardware the GPU renderer uses about a quarter of the
compositor CPU of the CPU renderer under damage (measured on Apple
Silicon; method and numbers:
[Backends and rendering](../scoot/backends.md#which-renderer-draws-the-frames)).

## The smallest build: CPU only

`cargo build --release -p scoot --no-default-features` (Nix:
`scoot-minimal`) builds the CPU renderer with no GPU code at all: no
GBM calls, no C stub, no libgbm reference, and no X11 code. Building it
needs no C compiler. In such a build `--renderer gpu` warns and keeps
the CPU renderer, and `auto` resolves to cpu with reason
`no-scanout-tier`.

Add a tier only when you need it: the default features (the GPU scanout
tier, on by default) or `--features xwayland`. System libraries per
feature, the toolchain floor and the offline build: [Package scoot
offline](./packaging.md).
