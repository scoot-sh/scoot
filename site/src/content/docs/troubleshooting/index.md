---
title: Symptom index
description: "Fixes by symptom, across every app — find it here, fix it there."
---

Something wrong? Find the symptom, follow the link. Start with the one
that looks most like yours. Each app's own
troubleshooting page has the full diagnosis; this index gets you to the
right one.

## No picture

- *Black screen / no output on `--tty`* → [Backends and rendering](../scoot/backends.md#which-drm-device---tty-drives): the startup error lists every device tried and why; a busy seat ("one client at a time") is the usual cause.
- *Nested window opens and instantly closes* → [First session](../start/first-session.md#start-scoot-in-a-window): closing the window *is* quitting.
- *Wallpaper never appears / image not filling* → [scootbg troubleshooting](../scootbg/troubleshooting.md).

## Keys and pointer

- *Key not working / Super does nothing nested* → [First session](../start/first-session.md#five-keys) and [Keybindings](../scoot/keybindings.md#change-one-binding): the host owns Super first inside a nested window.
- *A Fn key does nothing* → [Desktop hardware keys](../desktop/index.md#hardware-keys-and-desktop-actions): check the tool first (`brightnessctl get`, `wpctl get-volume`), then the bind, then whether scoot saw the key.
- *Volume keys die at the lock screen / holding volume steps once* → known gaps, [documented with the keymap](../desktop/index.md#hardware-keys-and-desktop-actions).
- *Pointer frozen but clicks answer ok* → [Rules an agent needs](../msg/index.md#rules-an-agent-needs): a client holds a pointer lock; a session lock ends the freeze.

## Config and reload

- *Scoot won't start after a config edit* → almost never the config: [failure semantics](../scoot/configure.md#failure-semantics) fall back to defaults and log, except `[tty] gpu` and `[renderer] backend = "gles"`, which are deliberate startup errors.
- *Reload changed nothing* → [Reloading](../scoot/configure.md#reloading-the-config): both lists empty means file and session agree; `tty.gpu`, `renderer.backend`, `xwayland.enabled` and output `mode` need a restart.
- *A bind doesn't fire / two binds collide* → [Keybindings](../scoot/keybindings.md#the-bind-grammar): unparseable binds skip quietly; colliding combos both drop.

## Outputs and monitors

- *Bar missing on the second monitor* → [scootbar troubleshooting](../scootbar/troubleshooting.md): `--outputs` naming, exclusive zones, and the `--check` diagnosis.
- *Replug reaches the wrong screen* → [Outputs](../scoot/outputs.md#moving-across-outputs): ids are never reused — `scoot msg outputs` lists the fresh ones; stepping binds never name an id.
- *Screens never dim / locker rejects the password* → [Desktop idle and lock](../desktop/index.md#idle-and-lock): unit running? PAM configured? Caps Lock?

## Agents and IPC

- *Socket not there / action refused / screenshot blank* → [IPC troubleshooting](../msg/troubleshooting.md): the nine bounds, what each costs, and the retry rules.
- *Typed text lands on the wrong window* → [Rules an agent needs](../msg/index.md#rules-an-agent-needs): window focus vs keyboard focus, and the `popup_grab` rule.
- *No popups / popups under fullscreen / DND stuck* → [Desktop notifications](../desktop/index.md#notifications): unit, `layer=overlay`, `makoctl mode -r`.

## Look and feel

- *Bar looks wrong after a look change* → [scootbar theming](../scootbar/theming.md): which keys the look writes, what wins per key.
- *X app has no window / XWayland didn't start* → [XWayland](../scoot/xwayland.md): package *and* switch, both loud when half-set.
