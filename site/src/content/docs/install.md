---
title: Install
description: Get scoot onto your machine with Nix, or build it from source.
---

Get a working `scoot` binary. Three paths, fastest first — pick one.

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
`aarch64-linux` to the public Cachix cache `scoot-sh`. Without it, Nix
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

The binaries land in `target/release/`.

## Check it worked

```sh
scoot --help
```

This prints the usage text and exits — no display needed. If it complains
about a missing library, re-check the `apt` line above; if it says
something about `--headless` on macOS, that is expected: on a Mac only the
`scootctl` remote-control client builds.

Next: [First session](./first-session.md) — log in, open a terminal, learn five keys.
