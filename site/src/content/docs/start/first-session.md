---
title: First run
description: "Run scoot in a window, open a terminal, and learn the three keys that run everything."
---

Run scoot for the first time — nested in a window, then full-screen on a
console — and do something useful in ten minutes. You need [a binary](./install.md) and a terminal (the examples use `foot`, scoot's default).

## Start scoot in a window

The safest first run is nested inside the Wayland desktop you already use
(GNOME, KDE Plasma, sway, …):

```sh
scoot --nested -- foot
```

A window opens with a terminal in it. Close the window to quit — nothing on
your real desktop changes.

> **Symptom:** the nested window opens and instantly closes.
> scoot printed why in the terminal you started it from. Usually it is a missing `$XDG_RUNTIME_DIR` or a host that never answered: see [Backends](../scoot/backends.md).

## Three keys, and a way out

`Super` is the Windows/Command key. Directions are vim's <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd>.
In the browser demo, use `Alt` wherever this page says `Super`: your
browser keeps `Super` for itself.

| Press | You get |
|---|---|
| <kbd>Super</kbd>+<kbd>Return</kbd> | A new terminal |
| <kbd>Super</kbd>+<kbd>h</kbd> / <kbd>Super</kbd>+<kbd>l</kbd> | Focus the column left / right |
| <kbd>Super</kbd>+<kbd>q</kbd> | Close the focused window |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>e</kbd> | Quit scoot |

Try it: open three terminals, then <kbd>Super</kbd>+<kbd>h</kbd> and
<kbd>Super</kbd>+<kbd>l</kbd> to walk between them.

> **Symptom:** `Super` does nothing, or your desktop's own shortcuts fire instead.
> Your host desktop owns `Super` first inside a nested window. Either use the keys that get through, or [rebind scoot's keys](../scoot/keybindings.md#change-one-binding) to something free.

## How the layout works

Windows sit in **columns**; columns form a **strip** that scrolls sideways.
A new window never covers an old one — the strip grows and the view follows:

```text
┌────────┐  ┌────────┐  ┌────────┐
│ term 1 │  │ term 2 │  │ term 3 │  ◀── you are here
└────────┘  └────────┘  └────────┘
◀────────── the strip scrolls ──────────▶
```

- <kbd>Super</kbd>+<kbd>j</kbd> / <kbd>Super</kbd>+<kbd>k</kbd> move *within* a column (up/down).
- <kbd>Super</kbd>+<kbd>1</kbd>…<kbd>9</kbd> jump to workspaces; each workspace is its own strip.
- <kbd>Super</kbd>+<kbd>r</kbd> cycles the focused column's width.

## Go full-screen (your whole session)

When the window-in-a-window feels right, log out and pick **scoot** on
your login screen (the NixOS setup and `install.sh --system` both put
it there). No entry there? Switch to a text console
(<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F3</kbd>), log in, and run:

```sh
scoot-session
```

That starts the whole session: the bar and every service you
installed. Only have the bare binary? `scoot --tty -- foot` runs the
compositor alone.

<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd> switch back to
your usual desktop at any time — they always win over scoot's own keys, so
you can never strand yourself.

## Learn 3 keys today

You already know the three that run everything: <kbd>Super</kbd>+<kbd>Return</kbd> (terminal), <kbd>Super</kbd>+<kbd>h</kbd>/<kbd>Super</kbd>+<kbd>l</kbd> (move between columns), <kbd>Super</kbd>+<kbd>q</kbd> (close).

Next: [Keybindings](../scoot/keybindings.md) — the full default map, and how to make it yours. Or [build your desktop](./compose.md) — bar, wallpaper, launcher and notifications.
