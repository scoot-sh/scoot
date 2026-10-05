---
title: "Probe Sunshine/Moonlight streaming of a real scoot session on the M2"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Probe Sunshine/Moonlight streaming of a real scoot session on the M2

Filed 2026-10-04 on the maintainer's ask about remote desktop methods.
Serves **daily-drive** (low-latency streaming of your own desktop).

## The gap

Sunshine's usual Wayland capture is `wlr-screencopy`, which scoot
deliberately does not offer (`docs/protocols.md` "Not implemented").
Sunshine can also capture from KMS (needs `CAP_SYS_ADMIN`) and inject input
through uinput, both independent of the compositor, so it may already work
on a `--tty` session. Nobody has tried, and Asahi's display controller
(`apple,dcp`) may not expose the framebuffers KMS capture reads.

## What to do

A probe, no compositor code: run Sunshine (nixpkgs, `services.sunshine` with
`capSysAdmin`) against a `scoot-test` login on the M2, connect Moonlight from
another machine, and record what works: KMS capture on the GPU and the dumb
tiers, which output, latency, input via uinput, the cursor, and behavior when
the session locks. Restore the M2 to the maintainer's default after. If KMS
capture fails on DCP, record the error and whether `ext-image-copy-capture`
support in Sunshine (upstream, or a scoot-sh fork per CLAUDE.md, never an
upstream PR from here) is the path, and file that.

## Not in this ticket

Implementing any capture protocol; VNC (`virtual-input-remote-control`).
