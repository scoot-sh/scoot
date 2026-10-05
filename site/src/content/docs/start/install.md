---
title: Install
description: "Get scoot onto your machine with Nix, or build it from source — including which build you need."
---

Get a working `scoot` binary. Pick your build first (one command), then
your path below — or [take the whole desktop](./../desktop/index.md) and
skip choosing piece by piece.

## Which build do I need?

Two axes, decided independently: **CPU or GPU rendering**, and **whether
you need X11 apps**. Four prebuilt packages cover the combinations (all
in the Cachix cache, [below](#skip-the-compile-the-binary-cache)):

| Package | Renders with | Needs | Pick it when |
|---|---|---|---|
| `scoot` (default) | pixman on the CPU; no GPU at all | nothing | VM, webtop/container, no `/dev/dri`, nested-only use, or unsure |
| `scoot-gpu` | GPU scanout under `--tty`, dma-buf handoff under `--nested` | a DRM render node + Mesa/GBM driver, and a real seat for `--tty` | daily-driving on real hardware (laptop/desktop, Intel/AMD/Apple Silicon) |
| `scoot-xwayland` | as `scoot` | as `scoot`, plus X11 apps to run | you need X apps but no GPU path |
| `scoot-gpu-xwayland` | as `scoot-gpu` | as `scoot-gpu`, plus X11 apps to run | real hardware *and* X apps |

XWayland is its own axis for a reason: it adds ~344 MiB of closure (all
but ~12 MiB of it Xwayland's own, which links Mesa) and a trust decision
— any X client can keylog and read other X clients by design. Take it
only if you run X apps. (More in [XWayland](../scoot/xwayland.md).)

One visible side effect of the two `-xwayland` packages: their wrapper
leaves the process name as `.scoot-wrapped`, not `scoot` — so `pgrep -x
scoot` finds nothing there; use `pgrep -x .scoot-wrapped`.

### The one-command check

```sh
ls /dev/dri/renderD*
```

- **No output (no such directory, no matches)** → there is no GPU to use.
  Take the default `scoot`. This is the VM, webtop and container case,
  and it is a first-class configuration, not a degraded one: pixman is
  the default renderer and GPU-free operation is a hard requirement, not
  a fallback tier.
- **A render node listed (e.g. `renderD128`)** → a GPU exists. On real
  hardware with Intel/AMD graphics or Apple Silicon under Asahi Linux,
  take `scoot-gpu`. On a VM the node is usually software (llvmpipe) —
  there `gles` is several times *slower* than pixman, so stay on the
  default unless you measured otherwise.

Confirm what you actually got from scoot's own startup log: with
`--renderer gles` a working GPU prints `the GLES renderer is up`
with `device=/dev/dri/renderD128 software=false`. Trust that line over
the flag name — a device node backed by a software driver answers
`software=true`, and then you are rendering in software anyway.

What the GPU build buys, measured on an Apple M2 under Asahi Linux:
**4–5x less compositor CPU under damage** (20.8% of a core → 4.3% under
large-damage pointer motion; 52.0% → 14.0% under a full relayout), the
same pixels, ~0.2 W less power, 7–16 MB more RSS, and no measurable CPU
difference at idle. What it costs: the EGL drivers must come from your
system (NixOS: `hardware.graphics` enabled), and `--tty` scanout needs
the real seat.

> **Symptom:** `--renderer gles` exits at startup naming EGL devices.
> That is the intended behavior, not a bug to work around: a wrong
> `--renderer gles` is a **startup error, never a silent downgrade** —
> scoot names each failure and points back at `--renderer pixman`, which
> needs no GPU at all. There is **no automatic fallback** to the
> CPU tier. Either drop the flag (stay on pixman) or fix the cause: the
> GPU build installed, drivers present, render node visible.

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
work with `nix profile add` and `nix run` below — swap `scoot` for
`scoot-gpu` (or a `-xwayland` variant) wherever it appears.

## Try it without installing

If you have Nix, run scoot straight from the flake. Nothing is installed:

```sh
nix run github:scoot-sh/scoot -- --nested -- foot
```

This opens scoot in a window on your current desktop with a terminal in it.
Close the window to quit. (Why `foot`? It is scoot's default terminal —
install it too, or name another one. More in [First session](./first-session.md).)

## Install with Nix

scoot runs on Linux. With [Nix](https://nixos.org):

```sh
nix profile add github:scoot-sh/scoot#scoot github:scoot-sh/scoot#scootctl \
  github:scoot-sh/scoot#scootbar
```

> **Symptom:** `nix: command 'nix' not found`, or your Nix says `nix profile install` instead of `nix profile add`.
> Older Nix calls the subcommand `install` — same command, old name. If Nix itself is missing, install it from [nixos.org](https://nixos.org/download/) first.

### Skip the compile: the binary cache

Every merge to `main` pushes built binaries for `x86_64-linux` and
`aarch64-linux` to the public Cachix cache `scoot-sh` — all four `scoot`
variants above, plus `scootctl`, `scootbar` and `scootbg`. Without it, Nix
compiles Smithay and scoot's crates on your machine (minutes); with it, you
download. Opt in explicitly in your Nix configuration — NixOS
(`configuration.nix`):

```nix
nix.settings = {
  extra-substituters = [ "https://scoot-sh.cachix.org" ];
  extra-trusted-public-keys = [
    "scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo="
  ];
};
```

Per user (`~/.config/nix/nix.conf`), the same two lines:

```ini
extra-substituters = https://scoot-sh.cachix.org
extra-trusted-public-keys = scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo=
```

What this trusts: binaries built by CI from reviewed merges to `main`. A
substituter can serve any store path your Nix asks for, so this trusts CI's
builds the way installing the flake already trusts its source.

## Build from source

On Debian or Ubuntu (Rust 1.87 or newer):

```sh
sudo apt install pkg-config libwayland-dev libxkbcommon-dev libinput-dev \
  libdrm-dev libdisplay-info-dev libseat-dev libudev-dev libpixman-1-dev \
  libgbm-dev libegl-dev libdbus-1-dev
git clone https://github.com/scoot-sh/scoot && cd scoot
cargo build --release -p scoot -p scootctl -p scootbar
```

The binaries land in `target/release/`. For the GPU build, add the
feature: `cargo build --release -p scoot --features gpu-scanout` (plus
`xwayland` for X apps).

## Check it worked

```sh
scoot --help
```

This prints the usage text and exits — no display needed. If it complains
about a missing library, re-check the `apt` line above; if it says
something about `--headless` on macOS, that is expected: on a Mac only the
`scootctl` remote-control client builds.

Next: [First session](./first-session.md) — run scoot, open a terminal, learn five keys. Or skip the piece-by-piece path: [the scoot desktop](./../desktop/index.md) is one switch plus a look.
