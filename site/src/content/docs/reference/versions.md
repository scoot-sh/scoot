---
title: Versions and compatibility
description: "What each binary's version means, which versions work together, and how to check."
---

Every binary prints its own version and the protocol it speaks. When
something disagrees after an upgrade, this page says which half to look
at.

## Ask each binary what it is

```sh
scoot --version
scootbg --version
scootbar --version
```

Each prints one line and exits without starting anything:

```text
scoot 0.1.0 (ipc protocol 10)
scootbg 0.1.0 (protocol 1)
scootbar 0.1.0
```

`scoot 0.1.0` is the compositor's own version; `(ipc protocol 10)` is
the control protocol `scoot msg` speaks, so a client checks compatibility
before connecting. `scootbg` names its wallpaper protocol the same way.
`scootbar` carries no protocol number: it speaks standard Wayland
protocols, plus an optional compositor feed guarded by the IPC protocol
number.

The compositor and the wallpaper daemon move together (one version
line); the bar moves on its own, so the bar and the compositor can each
release without the other.

## Which versions work together

- **Bar against compositor:** any pair works as long as both speak the
  same Wayland protocols. The optional compositor feed refuses cleanly
  past an IPC protocol bump instead of misbehaving.
- **Compositor against wallpaper daemon:** the same protocol is
  required, and `scoot` checks it on every reload before sending the
  section. A version mismatch only warns: the section is still applied
  by the running build. A daemon too old to know the handshake is
  refused with the fix (`scootbg kill`, then reload).
- **Client against compositor:** compare the `(ipc protocol N)` in
  `scoot --version` with the running session's. A mismatch means the
  client's verbs may not match the session's grammar; upgrade the older
  half.

## After an upgrade looks wrong

> **Symptom:** the wallpaper did not change after upgrading one half.

Run `scootbg --version` beside `scoot --version`. If the protocols
differ, the compositor's log names the refusal; kill the old daemon
(`scootbg kill`) and reload, which starts the new one.

> **Symptom:** the bar's optional compositor feed stopped updating.

The feed follows the IPC protocol number. Upgrade the bar beside the
compositor, or turn that feed's module off until they match.

Releases are cut per package (`scoot-v0.2.0`, `scootbar-v0.3.0`), each
with its own changelog section, so an upgrade note names exactly the
binary it changes.
