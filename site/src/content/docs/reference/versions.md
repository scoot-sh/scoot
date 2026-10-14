---
title: Versions and compatibility
description: "What a scoot version means, which builds work together, and how to check."
---

scoot has one version for every binary, and every binary prints it with
the protocol it speaks. When something disagrees after an upgrade, this
page says which half to look at.

## Ask each binary what it is

```sh
scoot --version
scootbg --version
scootbar --version
```

Each prints one line and exits without starting anything:

```text
scoot 0.2.0 (ipc protocol 10)
scootbg 0.2.0 (protocol 1)
scootbar 0.2.0
```

`0.2.0` is the release, the same for all three: one tag, `v0.2.0`,
releases every binary. `(ipc protocol 10)` is the control protocol
`scoot msg` speaks, so a client can check compatibility before relying
on newer requests. `scootbg` names its wallpaper protocol the same way.
`scootbar` prints no protocol number: it speaks standard Wayland
protocols to the compositor, and its own control protocol to
`scootbar msg` (both halves come from one install).

A build that is not a release (from a git checkout or an Arch `-git`
package) adds where it came from:

```text
scoot 0.2.0+14.g615608c (ipc protocol 10)
```

That reads "14 commits past v0.2.0, at commit `615608c`". The protocol
number is what decides compatibility, not the suffix. Before a version
moves, the maintainer reviews it in a release pull request: a breaking
change bumps the minor while scoot is below 1.0, anything else the
patch.

## Which versions work together

- **Everything from one release:** the supported way. Install and
  upgrade every binary together, at one version.
- **Bar against compositor:** any pair works as long as both speak the
  same Wayland protocols. The bar's one compositor action (`quit`) is a
  single fixed request every IPC protocol so far understands.
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

> **Symptom:** one binary prints a different version from the others.

Something was upgraded on its own. Upgrade the rest to the same
release, then restart the session so every daemon
runs the new build. Each release's changelog section lists what moved,
grouped by component, so an upgrade note names the binary it changes.
