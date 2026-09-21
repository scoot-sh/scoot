---
title: "`--tty` hotplug follow-up: confirm the two unreproduced paths on real hardware"
status: "open"
area: "core"
priority: "medium"
blocked: "needs vfkit/laptop hardware — neither path reproduces on the QEMU dev VM"
---

# `--tty` hotplug follow-up: confirm the two unreproduced paths on real hardware

Filed as gh issue #48 (2026-09-16 — read it for the full shape, not a
plan). PR #51 landed the `--tty` hotplug following (udev monitor,
connector re-choice, re-modeset); see
`../resolved/tty-drm-hotplug-done.md` for what shipped and what it
proved on the QEMU dev VM. This entry tracks exactly what is left: the
issue stayed open because two paths could not be reproduced there.

1. **A new mode list on the same connector** (vfkit host-window
   rescale/move offering a new preferred mode + list). The code path is
   the same re-choice the VM proved for connector *loss*; what wants
   confirmation is the list actually changing under a live session and
   the session following to the new preferred (or `--mode`) size, with
   `wl_output.mode`/`done` reaching clients.
2. **Falling back to a *different* connector** (unplug the one it is on
   with another `Connected` one present). Single-output invariant holds
   — the session must switch connectors, not add one — and must not go
   black.

Both want the vfkit/laptop hardware that filed the issue (or equivalent
two-connector/relayout-capable hardware), with the exact commands,
`wlr-randr` before/after, and screenshot proof recorded — not a
paraphrase. If a path fails on hardware, it becomes its own fix ticket;
if both confirm, close this entry with the evidence and close #48.
