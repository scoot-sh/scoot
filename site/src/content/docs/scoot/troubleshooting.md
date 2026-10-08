---
title: scoot troubleshooting
description: "Compositor symptoms and their fixes — picture, keys, config, outputs."
---

## No picture

- *Black screen on `--tty`.* Read the startup error: it lists every
  DRM device tried and why each was rejected. "Seat takes one client
  at a time" means another compositor holds the seat — no device
  choice fixes that. See [which device runs](./backends.md#which-drm-device---tty-drives).
- *Nested window opens and instantly closes.* Closing the window *is*
  quitting — re-run and use `Super+Shift+e` only to leave. See
  [First session](../start/first-session.md#start-scoot-in-a-window).
- *A monitor stays dark after plug.* The output gets an id per plug —
  `scoot msg outputs` shows whether scoot sees it. Only what the kernel
  reports is followed: a disconnect the kernel never reports leaves
  scoot driving a screen that is no longer there.
- *`--tty` display stays dark after a VT switch back.* The session is
  usually alive: `scoot msg outputs` answers with `live: false`, and the
  log names the holder scoot could not take DRM master from. Switch VTs
  away and back -- each return retries, and any one of them may win the
  race. If it never recovers, restart the session (a fresh start
  re-acquires master through the seat daemon). See [VT
  switching](./backends.md#hotplug-vt-switching-captures).

## Keys

- *`Super` does nothing nested.* The host owns Super first inside a
  nested window — use the keys that get through, or rebind to
  something free ([change one binding](./keybindings.md#change-one-binding)).
- *A bind silently does nothing.* Unparseable binds skip with a
  warning (check the log); two combos resolving to the same key both
  drop. See [the bind grammar](./keybindings.md#the-bind-grammar).
- *VT keys don't switch.* Under `--headless`/`--nested` there is no VT
  to switch to — those binds exist only on `--tty`, where they always
  win.

## Config

- *Scoot won't start after an edit.* It still starts: everything but
  `[tty] gpu` and `[renderer] backend = "gles"` falls back to defaults
  and logs. Those two are deliberate startup errors naming the key.
  See [failure semantics](./configure.md#failure-semantics).
- *Reload changed nothing.* Both lists empty means file and session
  agree. `tty.gpu`, `renderer.backend`, `xwayland.enabled` and output
  `mode` need a restart — each refusal says so.

## Windows and outputs

- *Dialog tiles / tiled dialog.* Float is decided at map time from
  dialog hints, parent, and fixed size — plus your rules, which always
  have the last word. See [Windows](./windows.md).
- *X app has no window.* Package *and* switch: without the `xwayland`
  build the knob warns and runs Wayland-only. See
  [XWayland](./xwayland.md).
- *Wrong scale / blurry X apps.* Scale is per output, told to windows
  on move; X apps draw at `ceil(scale)` unless `fractional = "light"`.
  See [Outputs](./outputs.md) and [XWayland](./xwayland.md).
- *`--renderer gles` exits naming EGL.* Intended: no silent downgrade,
  no automatic fallback. Drop the flag or fix the cause — see
  [Backends](./backends.md#which-renderer-draws-the-frames).
- *VNC connects but input does nothing.* Remote control is opt-in
  (`[virtual_input] enabled`, then restart), and nothing virtual
  delivers while locked. Keybindings over VNC need the second switch
  (`[virtual_input] binds`, then restart). See [Remote desktop](./remote-desktop.md).
