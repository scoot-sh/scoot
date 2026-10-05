---
title: A compositor that scrolls
description: A scrolling-tiling Wayland compositor that runs without a GPU.
template: splash
hero:
  title: A compositor that scrolls
  tagline: scoot tiles your windows in sideways-scrolling columns, runs without a GPU, and answers to scripts and agents over IPC.
  image:
    html: '<img src="./vinyl-sunset-preview.png" class="hero-image" alt="scoot wearing the vinyl-sunset look — a floating translucent bar over two translucent terminal columns, the sunset illustration behind" />'
  actions:
    - text: Install scoot
      link: install/
      icon: download
    - text: Take it for a spin
      link: first-session/
      icon: rocket
      variant: minimal
---

Windows sit in columns; columns scroll sideways. New windows never cover old
ones — the strip just grows, and you move along it.

## Start here

- [Install](./install.md) — the flake, the binary cache, or one `nix run` to try it.
- [First session](./first-session.md) — log in, open a terminal, learn five keys.
- [Keybindings](./keybindings.md) — the full default map and how to rebind it.

## Why scoot

- **Scrolling columns, not a grid.** Focus follows position; nothing ever hides.
- **No GPU required.** The pixman renderer draws on the CPU, so scoot runs on a webtop container, a VM, and real hardware alike.
- **Scriptable to the core.** Every action in [Keybindings](./keybindings.md) is also an IPC action an agent can send — windows, screenshots, input, the lot.

*Home-page art: the [vinyl-sunset look](https://github.com/scoot-sh/scoot/tree/main/docs/examples/vinyl-sunset) (screenshot; the wallpaper illustration stays under its [Pixabay license](https://pixabay.com/service/license/)).*
