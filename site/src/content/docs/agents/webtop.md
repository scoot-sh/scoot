---
title: Webtop
description: "Run scoot nested inside a linuxserver webtop container — GPU-free by design."
---

scoot runs inside a [linuxserver webtop](https://docs.linuxserver.io/images/docker-webtop/)
container nested in the host compositor — no GPU or seat needed,
because the default build requires neither. The container's desktop init
execs a session script with the environment already set; start scoot
nested in the host compositor with the same session script:

```sh
#!/bin/sh
# /defaults/startwm.sh (a linuxserver convention, outside this repo): the container's desktop init
# execs this with the session environment already set. Start scoot nested
# in the host compositor, with the same session script:
exec scoot --nested -- /path/to/session.sh
```

Pull a glibc-based tag: the installer supports glibc only, while the
default tag (`latest`) is Alpine/musl — `debian-xfce` is one that works.
Run the container with `-e PIXELFLUX_WAYLAND=true`: without it the image
boots X11 and there is no Wayland socket to nest into.

> **Symptom:** `--nested` stops after 10 seconds with "the host compositor
> did not answer the startup registry request", and the container's
> `wayland-1` socket is gone afterwards. **Cause:** in this release scoot
> binds `wayland-1` itself, unlinking the host's live socket, then connects
> to itself. Do not run `--nested` inside a webtop until the fix lands
> (backlog: `nested-steals-host-wayland-socket`). `scoot --headless` works
> there as a stand-in.

scoot does not wait on the script and restarts nothing that dies —
supervision is the container's job (there is no systemd here), and the
D-Bus activation half is the session script's (see [the portal
setup](../scoot/index.md#starting-a-session), s6 flavor).

`auto` resolves to pixman here and GPU-free
operation is a hard requirement, so there is no driver to install and
no device to pass through. The one-command check on
[Install](../start/install.md#which-build-do-i-need) answers "no
render node" here — take the default `scoot`.
Drive it over [IPC](../msg/index.md) and read it back with
[Screenshots](../msg/screenshots.md), like a headless
agent session.
