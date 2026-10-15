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

The only remaining knob for X11 apps is **whether you turn them on**,
not which package you install: every `scoot` already carries the
`xwayland` build feature, and only if you run X11 apps should you pay
Xwayland's own ~344 MiB of closure (all but ~12 MiB of it Xwayland's
own, which links Mesa) and its trust decision — any X client can read
other X clients' keys by design. To run X11 apps: install the `Xwayland`
binary beside scoot (the Nix package already has it on `PATH` via its
wrapper) and turn the knob on (`--xwayland` or `[xwayland] enabled`).
(More in [XWayland](../scoot/xwayland.md).)

One visible side effect of the wrapper: it leaves the process name as
`.scoot-wrapped`, not `scoot` — so `pgrep -x scoot` finds nothing there;
use `pgrep -x .scoot-wrapped`.

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
Leave the default (omit `package`): there is only one package now.

```nix title="configuration.nix"
programs.scoot = {
  enable = true;
  session.enable = true;
};
```

`inputs` and `pkgs` need `{ inputs, pkgs, ... }:` at the top of
`configuration.nix` (the Install page's `flake.nix` passes `inputs`
down through `specialArgs`).

The same name works with `nix profile add` and `nix run`.

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

`cargo build --release -p scoot --no-default-features` builds the CPU
renderer with no GPU code at all: no GBM calls, no C stub, no libgbm
reference, and no X11 code. Building it needs no C compiler. In such a
build `--renderer gpu` warns and keeps the CPU renderer, and `auto`
resolves to cpu with reason `no-scanout-tier`. There is no published
minimal Nix package: the minimal build is a documented cargo command
only.

The default features are the GPU scanout tier and `xwayland`, both on
by default. System libraries per feature, the toolchain floor and the
offline build: [Package scoot offline](./packaging.md).
