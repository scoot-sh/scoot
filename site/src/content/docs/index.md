---
title: A compositor that scrolls
description: "A scrolling-tiling Wayland compositor that runs without a GPU."
template: splash
hero:
  title: A compositor that scrolls
  tagline: scoot tiles your windows in sideways-scrolling columns, runs without a GPU, and answers to scripts and agents over IPC.
  image:
    html: '<img src="./vinyl-sunset-preview.png" class="hero-image" alt="scoot wearing the vinyl-sunset look — a floating translucent bar over two translucent terminal columns, the sunset illustration behind" />'
  actions:
    - text: Get the scoot desktop
      link: desktop/
      icon: download
    - text: Take it for a spin
      link: start/first-session/
      icon: rocket
      variant: minimal
---

Windows sit in columns; columns scroll sideways. New windows never cover old
ones — the strip just grows, and you move along it.

## Three ways in — pick one

- **[The scoot desktop](./desktop/index.md)** — the primary path. One switch
  plus a look (`programs.scoot.desktop.enable` + `look`) gets the full
  lightweight desktop: compositor, bar, wallpaper, greeter, idle and lock,
  notifications, hardware keys. Start here unless you know you want less.
- **[Just the compositor](./start/install.md)** — first-class, bring the rest
  yourself. Install any `scoot` build, wire your own bar, launcher and
  shell, on NixOS or anywhere else. The desktop profile stays optional by
  design.
- **[Agents, headless and webtop](./agents/index.md)** — drive scoot from
  code. The IPC socket, screenshots, `--help --json`, `llms.txt` sets for
  every section, and the webtop image for containers.

New here? [Install](./start/install.md) (including **which build you need:
GPU or CPU**) → [First session](./start/first-session.md) (run scoot, open
a terminal, learn five keys) → [Keybindings](./scoot/keybindings.md) (the
full default map and how to make it yours).

## Why scoot

- **Scrolling columns, not a grid.** Focus follows position; nothing ever hides.
- **No GPU required.** The pixman renderer draws on the CPU, so scoot runs on a webtop container, a VM, and real hardware alike. A GPU build exists for real hardware — [picking takes one command](./start/install.md#which-build-do-i-need).
- **Scriptable to the core.** Every action in [Keybindings](./scoot/keybindings.md) is also an IPC action an agent can send — windows, screenshots, input, the lot.

*Home-page art: the [vinyl-sunset look](https://github.com/scoot-sh/scoot/tree/main/docs/examples/vinyl-sunset) (screenshot; the wallpaper illustration stays under its [Pixabay license](https://pixabay.com/service/license/)).*
