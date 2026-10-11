![scoot — a cat swiping three terminal windows sideways across the screen,
trailing motion lines](docs/assets/logo.png)

# scoot

[![CI](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml/badge.svg)](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml)

**scoot knocks your windows sideways.** It is a light Wayland compositor
that puts windows in columns on a strip that scrolls left and right. A
new window never squeezes the others. The strip just gets longer.

![scoot with the ginger-night look: a floating bar over two terminal columns, the cat-peeking wallpaper behind](docs/assets/ginger-night-preview.png)

## See it now

No install. Start the desktop in your browser with
[Docker](https://docs.docker.com/get-started/get-docker/):

```sh
docker run --rm -p 127.0.0.1:6080:6080 ghcr.io/scoot-sh/try-scoot
```

Open `http://127.0.0.1:6080` and click the desktop. Press `Alt+Return`
for a second terminal. The port stays on loopback because VNC has no
password. To stop, press `Ctrl+C` where you started the container.

On Linux with [Nix](https://nixos.org/download/), this instead opens
scoot in a window on your current desktop. It installs nothing:

```sh
nix shell --accept-flake-config github:scoot-sh/scoot nixpkgs#foot --command scoot --nested -- foot
```

Close the window to quit. If Nix starts to compile, your Nix does not
trust the cache yet: [add it](https://www.scoot.sh/start/install/#or-in-a-window-on-linux-nix).

## Pick your way in

| You want to… | Do this |
|---|---|
| Try it **in a browser**, with no install | `docker run` above, then open `http://127.0.0.1:6080`. [Install → Try it in your browser](https://www.scoot.sh/start/install/#try-it-in-your-browser) |
| Use it every day on **NixOS** | Add the flake and turn on `programs.scoot.desktop`. You get a bar, wallpaper, lock, notifications and a launcher. [Install → Use it every day](https://www.scoot.sh/start/install/#use-it-every-day) |
| Use it every day on **Debian, Ubuntu, Fedora, openSUSE or Arch** | Packages are coming. Today, build one from this repo in a few steps. [Install](https://www.scoot.sh/start/install/#use-it-every-day) |
| Keep it on **any Linux with Nix** | `nix profile add github:scoot-sh/scoot#scoot github:scoot-sh/scoot#scootbar github:scoot-sh/scoot#scootbg` |
| Drive it **from a script or an agent** | `scoot --headless -- foot`, then `scoot msg screenshot --out shot.png`. [Agents](https://www.scoot.sh/agents/) |
| Build a desktop **from the parts** | [Build your desktop](https://www.scoot.sh/start/compose/) |

## Why scoot

- **No GPU? No problem.** scoot draws with the CPU or the GPU, and both
  renderers are first-class. It runs on real hardware, in a VM with no
  3D, in a container, or inside another desktop. With the GPU build,
  `--renderer auto` picks the right renderer for the session, and
  `--renderer cpu` or `--renderer gpu` always overrides it.
  ([Builds and renderers](https://www.scoot.sh/reference/builds/).)
- **Agents can drive it.** One socket does everything computer use
  needs: type, click, take screenshots, list windows, and wait for the
  screen to settle. scoot's own tests drive it this way.
- **It stays out of the way.** An idle scoot does not wake up. The bar
  and the wallpaper daemon are just as frugal.
  ([Benchmarks](dev/benches/benchmarks.md).)

If you want the most mature scrolling compositor, use
[niri](https://github.com/niri-wm/niri). It is great, and it inspired
scoot.

## Five keys

`Super` is the Windows or Command key.

| Keys | Does |
|---|---|
| `Super+Return` | Open a terminal |
| `Super+h` / `Super+l` | Focus the column to the left / right |
| `Super+q` | Close the window |
| `Super+Shift+/` | Show every key |
| `Super+Shift+e` | Quit scoot |

All keys, and how to change them: [Keybindings](https://www.scoot.sh/scoot/keybindings/).

## Make it yours

scoot reads `~/.config/scoot/config.toml`. Every setting is optional,
and a mistake never stops scoot from starting. To get the defaults,
with comments:

```sh
scoot --print-default-config --write
```

`scoot msg reload` applies a change without a restart. Every option is
on [Configure](https://www.scoot.sh/scoot/configure/). Five ready-made
looks (black, light, dark, warm, chill) are on
[Theming](https://www.scoot.sh/scoot/theming/).

## What comes in the box

| Piece | What it does |
|---|---|
| `scoot` | The compositor, plus `scoot msg`, its control client |
| `scootbar` | A status bar: workspaces, clock, sound, network, battery, tray. It also works on sway, niri and Hyprland. |
| `scootbg` | A wallpaper daemon. Set `[wallpaper]` in the config and scoot starts it. |

Standard Wayland protocols mean the usual tools work: other bars,
launchers, notification daemons, screen sharing, screen lockers and
X11 apps (with the XWayland build). [Full list](https://www.scoot.sh/scoot/protocols/).

## Not yet

- **Monitor placement.** Monitors line up left to right. There is no
  position setting yet.
- **Changing display modes from `wlr-randr` or Settings.** Read-only for
  now. Set modes in the config file.
- **Some X11 extras.** X input methods (XIM) and X window icons.
- **A macOS compositor.** On a Mac, `scoot msg` drives scoot in a Linux VM.

## Docs

Everything else is on **[scoot.sh](https://www.scoot.sh/)**. What
changed is in [CHANGELOG.md](CHANGELOG.md), and what is next is in
[ROADMAP.md](ROADMAP.md). To build or contribute, start at
[dev/README.md](dev/README.md).

## License

MIT. See `NOTICE` for third-party attribution. scoot is built on
[Smithay](https://github.com/Smithay/smithay).
