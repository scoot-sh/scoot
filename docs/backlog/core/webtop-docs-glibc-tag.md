---
title: "webtop page names no glibc tag and the install page does not warn off Alpine/musl"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# webtop page names no glibc tag and the install page does not warn off Alpine/musl

Filed 2026-10-17 from the webtop field test (T-webtop). Serves **daily-drive**:
a stranger following Install -> webtop walks into a wall on the default image.

## The gap

1. `site/src/content/docs/agents/webtop.md` says "a linuxserver webtop container"
   but names no tag. The default tag (`latest` = XFCE Alpine) is musl, and
   `install.sh` refuses musl outright:
   `scoot-install: error: the prebuilt binaries need glibc (musl distros such as
   Alpine: build from source)`. Verified `lscr.io/linuxserver/webtop:debian-xfce`
   (Debian 13, glibc 2.41) works with the installer — name it (or ubuntu-xfce)
   as the tag the installer supports, and say Alpine tags are build-from-source-only.
2. The Install page's "Any Linux" tab lists supported distros but never says
   "glibc only / no Alpine/musl", so the musl refusal surprises. One sentence
   covers it.
3. Pipe noise: `curl -fsSL ... | sh` prints curl's
   `curl: (23) Failure writing output to destination` ABOVE the real
   `scoot-install: error:` line whenever sh exits early (the pipe closes).
   It reads as a download failure. Either silence curl's stderr past the
   script's own errors (careful: must not hide real network failures) or add
   one docs line that a `curl: (23)` above a `scoot-install: error:` is fallout,
   not the cause.

## What to do

- webtop.md: tag recommendation + one-line why (glibc vs musl), Wayland-mode
  env (`PIXELFLUX_WAYLAND=true`) already implied by "Wayland-capable" — make
  it explicit while here, since the default boot has no Wayland socket at all.
- install.mdx Any-Linux tab: "needs glibc 2.36+; Alpine/musl is build-from-source-only".
- Decide the curl-(23) fix (script vs docs) and do the smaller one.

## Not in this ticket

The nested socket-steal and foot-version bugs found in the same test (separate tickets).
