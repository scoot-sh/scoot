---
title: "qemu-test phase 3e: nh os switch races the greeter's user manager shutdown"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# qemu-test phase 3e: `nh os switch` races the greeter's user manager shutdown

Filed 2026-10-29. Serves **daily-drive**: a CI check that fails at random hides real regressions in the installer.

## The gap

scoot-iso#22 (the v0.6.11 pin) failed `qemu-test` once on head `81b82cd`
(job 123179833664) in phase 3e, "offline rebuilds on the installed system".
`nh os switch` printed `Activation (test) failed` and `user activation for
greeter failed`, with `Transaction for sockets.target/start is destructive
(... has 'stop' job queued ...)`. Re-running the failed job on the same head
passed. The same step passed in scoot-iso run 40793122078 and on `main`
earlier.

The reviewer's reading: systemd reloads the greeter's `systemd --user` units
while that user manager is still stopping after the ReGreet session, so the
queued stop collides with the start. It depends on timing between the login
and the next `nh os switch` in `scripts/qemu-test.sh`, not on content. #22
only changed the scoot pin. Not yet reproduced or proven.

## What to do

Wait in `scripts/qemu-test.sh` phase 3e until the greeter's user manager has
settled (its `user@UID.service` inactive, or no stop job pending) before
`nh os switch`, with a bounded wait that fails with a message naming what it
waited for. Do not add a blind retry that would hide a real activation
failure. Show the failure rate before and after if it can be provoked (a
loop of the phase in a booted installed system).

## Not in this ticket

The activation path itself (home-manager, NixOS modules): the failure is the
test's timing, unless the loop shows the race can also hit a user switching
configs right after a greeter login, in which case file that separately.
