---
title: knocks your windows sideways.
description: "A scrolling-tiling Wayland compositor that runs without a GPU."
template: splash
hero:
  title: knocks your windows sideways.
  tagline: A scrolling Wayland compositor. Windows tile in sideways-scrolling columns, it runs without a GPU, and agents can drive it over IPC.
  image:
    html: '<picture class="hero-art"><source type="image/avif" srcset="./hero-cat-960.avif 960w, ./hero-cat-1440.avif 1440w" sizes="(max-width: 1280px) 100vw, 1280px" /><source type="image/webp" srcset="./hero-cat-960.webp 960w, ./hero-cat-1440.webp 1440w" sizes="(max-width: 1280px) 100vw, 1280px" /><img src="./hero-cat-960.jpg" fetchpriority="high" decoding="async" alt="scoot — a grumpy ginger cat swats terminal windows sideways off a black background, under a huge white scoot wordmark" /></picture>'
  actions:
    - text: Get the scoot desktop
      link: desktop/
      icon: download
    - text: Take it for a spin
      link: start/first-session/
      icon: rocket
      variant: minimal
---

## Looks

Same cat, different wallpaper. Five looks ship with the desktop — picking
one themes the compositor, the bar and the wallpaper together (the
default is ginger-night):

<div class="looks">
<a href="./scoot/theming/"><img src="./looks/ginger-night-480.webp" alt="The ginger-night look: a floating translucent bar over two translucent terminal columns, the cat-peeking illustration behind" loading="lazy" decoding="async" /><span>ginger-night (default)</span></a>
<a href="./scoot/theming/"><img src="./looks/vinyl-sunset-480.webp" alt="The vinyl-sunset look: a floating translucent bar over two translucent terminal columns, a sunset illustration behind" loading="lazy" decoding="async" /><span>vinyl-sunset</span></a>
<a href="./scoot/theming/"><img src="./looks/moonrise-480.webp" alt="The moonrise look: a calm amber-on-navy desktop" loading="lazy" decoding="async" /><span>moonrise</span></a>
<a href="./scoot/theming/"><img src="./looks/music-desk-480.webp" alt="The music-desk look: a blue-on-paper desktop" loading="lazy" decoding="async" /><span>music-desk</span></a>
<a href="./scoot/theming/"><img src="./looks/radial-burst-480.webp" alt="The radial-burst look: a blue-on-plum desktop" loading="lazy" decoding="async" /><span>radial-burst</span></a>
</div>

## Three ways in

Pick one:

- **[The scoot desktop](./desktop/index.md)** — the primary path. One switch
  plus a look (`programs.scoot.desktop.enable` + `look`) gets the full
  lightweight desktop: compositor, bar, wallpaper, greeter, idle and lock,
  notifications, hardware keys. Start here unless you know you want less.
- **[Just the compositor](./start/install.md)** — first-class, bring the rest
  yourself ([build your desktop](./start/compose.md): bar, wallpaper,
  launcher, notifications). Install any `scoot` build on NixOS or
  anywhere else. The desktop profile stays optional by design.
- **[Agents, headless and webtop](./agents/index.md)** — drive scoot from
  code. The IPC socket, screenshots, `--help --json`, `llms.txt` sets for
  every section, and the webtop image for containers.

## Start here, in order

1. [Install](./start/install.md) — in your browser with Docker, or in a window with Nix.
2. [First run](./start/first-session.md) — run scoot, open a terminal,
   learn three keys.
3. [Build your desktop](./start/compose.md) — bar, wallpaper, launcher
   and notifications.
4. [Keybindings](./scoot/keybindings.md) — the full default map and how to
   make it yours.

## Why scoot

- **Scrolling columns, not a grid.** Focus follows position; nothing ever hides.
- **No GPU required.** The CPU renderer draws without a GPU, so scoot runs on a webtop container, a VM, and real hardware alike. A GPU build exists for real hardware — [picking takes one look](./start/install.md#which-build-do-i-need).
- **Scriptable to the core.** Every action in [Keybindings](./scoot/keybindings.md) is also an IPC action an agent can send — windows, screenshots, input, the lot.

*Home-page art: the project logo (`docs/assets/logo.png`). Look previews: [vinyl-sunset](https://github.com/scoot-sh/scoot/tree/main/docs/examples/vinyl-sunset) (the wallpaper illustration stays under its [Pixabay license](https://pixabay.com/service/license/)), moonrise, music-desk and radial-burst.*
