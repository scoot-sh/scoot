---
title: "scootgreet baseline: measure ReGreet + cage, and verify cage's GPU-free path and the tty2 escape"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M7"
---

# scootgreet baseline

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#goals-non-goals-and-the-case-for-it).
Serves **daily-drive**. Measurement only, and unblocked.

## Already known (#579 review, pinned nixpkgs `8ce4ef6c`)

- ReGreet's closure is 956.7 MiB; ReGreet ∪ cage is 1,018.5 MiB. cage stays the host, and cage
  alone is 380.2 MiB, so what scootgreet could remove is ReGreet's paths beyond cage: **638.4 MiB
  (166 paths)** at most. That is the package union: it leaves out greetd, dbus and
  accounts-daemon, and it is not marginal over the installed profile.
- cage 0.3.1 returns the app's exit status (`cage.c:111,658-660`) and has no layer-shell.

## What to do

On the dev VM and the M2, with today's `programs.scoot.greeter.enable`:

- RSS of each process at the login screen;
- time from greetd start to first frame, **including the slow case**: QEMU without KVM and with
  no GPU (the scoot-iso test's setup). The supervisor's startup deadline is set from this number;
- **the marginal closure over the installed desktop profile** that scootgreet would remove
  (ReGreet's paths beyond cage, minus what the profile already carries). The go condition in
  `greet-prototype` needs at least about 300 MiB here, or the ISO still over its cap;
- cage on a GPU-less dev VM: does it draw in software, and with what setting? **If it fails,
  that unblocks `greet-scoot-kiosk-spike`**;
- `Ctrl+Alt+F2` reaches a getty with greetd on tty1;
- whether ReGreet localizes its labels.

## Acceptance

A table in this entry with the SHA and the method, and the design updated where a fact differs.

## Evidence the review needs

Raw command output for each item.

## Not in this ticket

Any code.
