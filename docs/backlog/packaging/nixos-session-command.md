---
title: "NixOS module: session entry has no way to launch a shell (Exec is a fixed `scoot --tty`)"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# NixOS module: session entry has no way to launch a shell (Exec is a fixed `scoot --tty`)

Filed as gh issue #171 (read it — the disconnect between the two
modules, the real-consumer evidence with the hand-rolled entry; this
entry tracks it).

`nix/modules/nixos.nix` builds the login entry with a fixed
`Exec=${cfg.package}/bin/scoot --tty` — no option appends `-- COMMAND`,
so the greeter starts a bare compositor with nothing in it. Meanwhile
the home-manager module writes `scoot/session.sh` and tells you to
launch it with `scoot -- ~/.config/scoot/session.sh` — which the NixOS
entry never passes. A user setting both gets a script that is written,
executable, and ignored.

Fix shape: a session-command option on the NixOS module (name it
consistently with the HM side — e.g. `session.command` as a string or
argv list appended after `--`), defaulting to today's bare `--tty`
(explicit, documented) so existing configs don't change meaning. The
HM docs' launch line and this option must agree — update both sides to
reference each other. The issue's hand-rolled
`writeTextDir ... providedSessions` entry is the acceptance shape: what
it expresses must become expressible through the module.

Scope: NixOS module option + docs on both sides + content-check
coverage in `nix/tests.nix` (a session entry rendering with a command).
No compositor code. The never-strand rule applies: the entry stays
additive (alongside existing sessions), whatever the command.
