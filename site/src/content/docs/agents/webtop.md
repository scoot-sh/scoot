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

Pull a glibc-based tag: the installer needs glibc 2.36 or newer (see
[Install](../start/install.md#use-it-every-day)), while the
default tag (`latest`) is Alpine/musl — build from source there.
`debian-xfce` is one tag that works; `ubuntu-xfce` is glibc-based too.
Run the container with `-e PIXELFLUX_WAYLAND=true`: without it the image
boots X11 and there is no Wayland socket to nest into.

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
