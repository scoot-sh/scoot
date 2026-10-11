---
title: Builds and renderers
description: "Which scoot package to install: CPU or GPU rendering, XWayland, and how the renderer is chosen."
---

Most people need one build: `scoot`. It draws with the CPU renderer and
runs everywhere. This page is for everyone else.

## Which build do I need?

Two choices, made independently: **which renderer draws** (CPU or GPU),
and **whether you run X11 apps**. Four packages cover the combinations.
All four are in the binary cache
([Use the binary cache](../start/binary-cache.md)).

| Package | Draws with | Needs | Pick it when |
|---|---|---|---|
| `scoot` (default) | the CPU renderer; no GPU at all | nothing | VM, container, no `/dev/dri`, nested-only use, or unsure |
| `scoot-gpu` | the GPU renderer under `--tty`, dma-buf handoff under `--nested` | a DRM render node plus a Mesa/GBM driver, and a real seat for `--tty` | daily-driving on real hardware (laptop, desktop) |
| `scoot-xwayland` | as `scoot` | as `scoot`, plus X11 apps to run | you run X11 apps but need no GPU path |
| `scoot-gpu-xwayland` | as `scoot-gpu` | as `scoot-gpu`, plus X11 apps to run | real hardware *and* X11 apps |

The XWayland axis costs more than the GPU axis: the `-xwayland`
packages carry a ~344 MiB larger closure (all but ~12 MiB of it
Xwayland's own, which links Mesa) and a trust decision — any X client
can read other X clients' keys by design. Take one only if you run X11
apps. (More in [XWayland](../scoot/xwayland.md).)

One visible side effect of the two `-xwayland` packages: their wrapper
leaves the process name as `.scoot-wrapped`, not `scoot` — so `pgrep -x
scoot` finds nothing there; use `pgrep -x .scoot-wrapped`.

### Do I have a GPU?

One command:

```sh
ls /dev/dri/renderD*
```

- **No output (no such directory, no matches)** → no GPU to use. Take
  the default `scoot`. The VM and container case is first-class, not
  degraded: the CPU renderer is the default, and running with no GPU is
  a hard requirement, not a fallback tier.
- **A render node listed (for example `renderD128`)** → a GPU exists.
  On real hardware, take `scoot-gpu`. On a VM the node is usually
  software (llvmpipe) — there the GPU renderer runs several times
  *slower* than the CPU renderer, so stay on the default unless you
  measured otherwise.

### Say it in the config

The package knob is `programs.scoot.package` (home-manager and NixOS
alike; the desktop profile never sets it — the choice stays yours):

```nix
programs.scoot = {
  enable = true;
  # Real hardware with a GPU: take the scanout build.
  package = inputs.scoot.packages.${pkgs.system}.scoot-gpu;
  # ...plus X11 apps? Use scoot-gpu-xwayland instead.
};
```

Leave the default (omit `package`) for the CPU build. The same names
work with `nix profile add` and `nix run` — swap `scoot` for
`scoot-gpu` (or an `-xwayland` variant) wherever it appears.

## Which renderer draws the frames

`--renderer cpu|gpu|auto` (config: `[renderer] backend`, env:
`SCOOT_RENDERER`) picks what composites each frame. The default is
`cpu`, the CPU renderer. `auto` picks the best tier for the session and
never fails startup: the GPU renderer on real hardware under `--tty`,
the CPU renderer in VMs, containers, nested and headless sessions. An
explicit `--renderer cpu` or `--renderer gpu` always wins over `auto`.

Every session logs one line saying what was decided and why —
`requested=… source=… tier=… reason=…` with `renderer chosen` on it.
When the tier is not what you expected, read `reason=` there first.
Full rules, per-session table and symptoms:
[Backends and rendering](../scoot/backends.md#which-renderer-draws-the-frames).

On real hardware the GPU renderer uses about a quarter of the
compositor CPU of the CPU renderer under damage (measured on Apple
Silicon; method and numbers:
[Backends and rendering](../scoot/backends.md#which-renderer-draws-the-frames)).

## The smallest build: CPU only

The default Cargo features build the CPU renderer and link no GPU
library:

```sh
cargo build --release -p scoot
```

Add a tier only when you need it: `--features gpu-scanout` or
`--features xwayland`. System libraries per feature, the toolchain
floor and the offline build: [Package scoot
offline](./packaging.md).

## One binary, later

The `runtime-gbm` feature (an off-by-default spike) loads libgbm at run
time instead of linking it. If it becomes the default, there is one
build and `auto` picks the renderer — and the table at the top of this
page goes away.
