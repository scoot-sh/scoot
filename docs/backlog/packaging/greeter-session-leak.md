---
title: "Each greeter session leaves two dbus-daemons and a closing logind session behind"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Each greeter session leaves two dbus-daemons and a closing logind session behind

Filed 2026-10-05. Serves **daily-drive**: a desktop that leaks processes
on every login is one nobody can leave running for weeks.

## The gap

`programs.scoot.greeter` turns on nixpkgs' ReGreet module, whose greetd
command is `dbus-run-session cage -s -d -m last -- regreet`. When greetd
ends the greeter session (a login, a greeter crash, a greetd restart), the
session bus `dbus-daemon` that `dbus-run-session` started survives, and so
does ReGreet's AT-SPI bus `dbus-daemon`. The host keeps
`KillUserProcesses=false` (the NixOS default), so logind leaves both
processes in the scope. The scope stays `active (abandoned)` and the
session stays `closing` for good.

Measured on the maintainer's Asahi M2, 2026-10-05, uptime 7 days:

- `loginctl list-sessions`: 85 greeter sessions on seat0. All are
  `State=closing` except the live one (`ActiveSession=56001`).
- 137 processes owned by `greeter`: two `dbus-daemon`s per dead session,
  plus the greeter's user manager.
- One sample scope (`session-54170.scope`, 9 h old) holds the two
  daemons at 544K now, after a 204.7M peak while ReGreet ran.

Most of those sessions came from agent test logins in two days. Normal
use leaks one pair per login and per greeter restart.

## What to do

- Make the greeter's processes end with its session, scoped to the
  greeter user so users' own lingering processes (tmux, ssh agents) are
  untouched. Candidates, in order: logind `KillUserProcesses=yes` with
  `KillOnlyUsers=greeter`, which only kills the greeter's scopes (check
  the `manager_shall_kill` logic in the pinned systemd, and that it does
  not override an explicit user setting; `mkDefault` both). Or a
  greeter command that does not leave a bus behind. The greeter user
  already runs a systemd user manager with `dbus-broker`, so
  `dbus-run-session` may be redundant; if so, also stop the AT-SPI bus
  with `NO_AT_BRIDGE=1`, or say why accessibility needs it. Pick one
  and say why.
- Pin it in `nix/tests.nix`. Prove it live: count `greeter` processes
  and `closing` sessions before and after three login/logout cycles as
  `scoot-test` (seat0 free, nobody logged in). The count must not grow.
- Edge cases: a greeter crash loop (greetd restarts it), a login that
  fails authentication, and suspend/resume while the greeter is up.

## Not in this ticket

- A custom greeter (planned later, on top of ReGreet).
- Leaks in user sessions; this is the greeter user only.
