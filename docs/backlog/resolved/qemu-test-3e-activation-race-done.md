---
title: "qemu-test phase 3e: nh os switch races the greeter's user manager shutdown"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
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

## Resolution

Resolved 2026-11-07 by [scoot-iso#82](https://github.com/scoot-sh/scoot-iso/pull/82).

Phase 3e of `scripts/qemu-test.sh` now waits, before `nh os switch`, for the
greeter's user manager (`user@<greeter-uid>.service`) to be inactive or
failed with no job in `systemctl list-jobs`, polling for up to 60 s. On
expiry it fails with `GREETER-MANAGER-FAIL` naming the unit, its state and
its jobs. It waits for inactive rather than for "no stop job" because
logind keeps the manager up for `UserStopDelaySec` after the last session
ends before queueing the stop, so "active, no job" is the racy window. It is
a precondition wait, not a retry: the activation runs once, so a real
activation failure still fails.

Evidence limits: the race was seen once in the field and never reproduced,
so there is no before/after failure rate. The snippet was checked with
`bash -n` and against stubbed `systemctl`/`id` (settled passes, a pending
stop job or a still-deactivating unit fails with the message); the PR's CI
run (head `d9595e0`) passed `qemu-test` and `qemu-test-online`, and its
log shows the manager was already inactive (`user@999.service`), so the wait
was a no-op there and the contended path was not exercised. A green run does not prove a timing race
fixed. Whether the same race can hit a user switching configs right after a
greeter login is untested and not filed.
