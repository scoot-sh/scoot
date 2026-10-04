---
title: "scoot-session ends the login when the user manager re-executes"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# scoot-session ends the login when the user manager re-executes

Filed 2026-10-04. Serves **daily-driving**: any `nh os switch` can log the
user out of their compositor session, orphaning `scoot.service` off-screen.

## The gap

2026-10-04, maintainer's Asahi M2 (greetd + ReGreet): an `nh os switch`
ended the logged-in scoot session. Journal (condensed): switch
re-executes the greeter's user manager, then at `15.20` the user's
manager (`systemd[1873]: Reexecuting.`), back at `15.54`. At `15.45` the
login session's scope ends, the greeter restarts, scoot logs
`drm commit/page flip failed` — while `scoot.service` stayed active,
orphaned, until stopped by hand (a next login would refuse as "already
running").

Cause, verified: `resources/scoot-session` polls
`systemctl --user --quiet is-active "$SERVICE"` once a second (wait loop;
same shape in the readiness loop). `is-active` exits nonzero not only for
"inactive" but when the manager cannot be asked at all — measured on the
dev VM (systemd 261.2): known-inactive unit reads `inactive`/rc 3,
unknown unit `inactive`/rc 4, unreachable manager rc 1
(`Failed to connect to user scope bus`). One poll inside the re-exec
window (~0.3 s here; every NixOS switch re-execs every user manager, so
every switch rolls these dice) reads as "scoot stopped": the launcher
exits, and its `cleanup` (`start scoot-shutdown.target`) fails in the
same window, so the compositor is never stopped. The v261
`systemctl(1)` Exit-status section says the LSB-code mapping "is
imperfect, so it is better to not rely on those return values but to
look for specific unit states and substates instead" — i.e. `show
-p ActiveState`, whose nonzero rc means "could not ask".

## What to do

Tell "the service is not active" from "the manager could not be asked"
everywhere the launcher asks (readiness loop, wait loop, refuse-or-heal
check, cleanup's `start scoot-shutdown.target`, its `reset-failed`/`stop`
calls): only `show -p ActiveState --value` returning a down state ends
the session; an unanswerable manager is retried, bounded, with a log
line; cleanup retries until the manager answers (bounded) so the
compositor is never orphaned. If the manager stays unreachable past the
bound it went away with the session (logout/shutdown), so the launcher
exits and frees the liveness lock instead of refusing the next login
forever. Pin each case in the stub harness with a "cannot connect" rc 1
window, failing on current main, and live-proof with
`systemctl --user daemon-reexec` in a loop around a real `scoot-session`
run on the dev VM.

## Not in this ticket

A real greeter run of the fixed launcher (brief forbids touching the
dev VM's display manager); the launcher-startup-exactly-during-reexec
race (degrades at once to bare `scoot --tty`, fail-safe: the session
still runs, nothing is killed or orphaned).
