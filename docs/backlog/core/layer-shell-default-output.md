---
title: "scoot: put an output-less layer surface on the pointer's output, as a new window"
status: "open"
area: "core"
priority: "low"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
---

# scoot: default output for layer surfaces

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#keyboard-first-ux). Serves
**daily-drive** (a launcher or notification opens where the user is looking).

## The gap

A layer surface that names no output lands on the primary output
(`crates/scoot/src/compositor/layer_shell.rs:228-239`), while a new window opens on the
pointer's output (`site/src/content/docs/scoot/protocols.md:1017-1021`). fuzzel and mako hit
this today on two monitors; scootlaunch and scootnotify would too.

## What to do

Resolve a null output to the pointer's output at the surface's initial configure, as windows
do; document it on the protocols page (the current text says the first output, the
compositor's choice).

## Acceptance

A two-output headless test: pointer on output 2, an output-less layer surface maps on output 2;
existing layer-shell tests unchanged otherwise.

## Not in this ticket

Moving a mapped surface when the pointer moves later.
