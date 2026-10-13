---
title: "scootgreet baseline: measure ReGreet + cage, and verify cage's exit status, GPU-free path and the tty2 escape"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# scootgreet baseline

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#goals-non-goals-and-the-case-for-it).
Serves **daily-drive**. Nothing in this repo has measured the current greeter, and the
no-stranding design rests on three unverified facts.

## What to do

On the dev VM and the M2, with today's `programs.scoot.greeter.enable`:

- ReGreet + cage + `dbus-run-session` closure (full and marginal), RSS of each process at the
  login screen, time from greetd start to first frame;
- cage's exit status when its child exits 0, non-zero and by abort (the wrapper keys on it);
- cage on a GPU-less dev VM: does it draw in software, and with what setting;
- `Ctrl+Alt+F2` reaches a getty with greetd on tty1 (nixpkgs disables only `autovt@tty1`);
- whether cage advertises `wlr-layer-shell` (scootgreet uses `xdg_toplevel` either way).

## Acceptance

A table in this entry with the SHA and the method; the design updated where a fact
differs.

## Evidence the review needs

Raw command output for each item; closure sizes from `nix path-info -S`.

## Not in this ticket

Any code.
