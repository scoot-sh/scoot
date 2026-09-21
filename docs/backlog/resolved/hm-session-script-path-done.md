---
title: "home-manager: sessionScript is not written next to the config when configFile is overridden — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# home-manager: sessionScript is not written next to the config when configFile is overridden — RESOLVED

## What it said

Filed as gh issue #174. In `nix/modules/home.nix`, `configFile` is a
settable option but the session script's path was hardcoded to
`xdg.configFile."scoot/session.sh"`. Both the module comment ("written
executable next to the config") and the `docs/nix.md` options table
("written executable to `scoot/session.sh` next to the config") claimed
a relationship that didn't hold: set
`configFile = "myscoot/custom.toml"` and the config lands in
`~/.config/myscoot/` while the script stays in `~/.config/scoot/` — a
script written somewhere the documented launch line doesn't point is a
broken session. `nix/tests.nix`'s `hmRelocated` case never set
`sessionScript`, so the pairing was untested. Either fix direction was
left open: derive the script path from `configFile`, or document the
fixed path.

## Resolution

**Direction (1): make the claim true.** The module now derives the
script path from `configFile`'s directory:

```nix
scriptPath =
  let
    d = builtins.dirOf cfg.configFile;
  in
  if d == "." || d == "" then "session.sh" else "${d}/session.sh";
```

and `xdg.configFile.${scriptPath}` replaces the hardcoded
`"scoot/session.sh"`. "Next to the config" is the better invariant: a
relocated config with its script beside it keeps working when launched
by relative expectation, while a fixed path silently splits the pair —
exactly the user-facing harm (broken session) this ticket is about.

Direction (1) was preferred only after enumerating what `configFile`
can legally be, per the ticket's fallback condition. The option is
`lib.types.str`, documented as relative to `$XDG_CONFIG_HOME`, consumed
as an `xdg.configFile` key:

- `"scoot/config.toml"` (default) → `dirOf` answers `"scoot"` → script
  stays `scoot/session.sh`: the default path is unchanged (proven
  byte-identical below, not assumed).
- Relocated subdir (`"myscoot/custom.toml"`) → `"myscoot/session.sh"`.
- Nested path (`"a/b/c.toml"`) → `"a/b/session.sh"`.
- Bare filename (`"config.toml"`) → `dirOf` answers `"."` (probed, not
  assumed), special-cased to `"session.sh"` at the config root — without
  the guard the key would be the nonsense `"./session.sh"`.
- Absolute paths are already out of contract on the config half (an
  `xdg.configFile` key must be relative), so they stay user error here
  too — no new failure mode, no new assertion.

No fallback to direction (2) was needed: `dirOf` handles every legal
shape, including the degenerate `"."`/`""` cases.

Docs state the exact rule now, not the vague claim: the option
description reads "written executable beside the rendered config at
`<dirOf configFile>/session.sh` (`scoot/session.sh` with the default
`configFile`)", the module comment points at `<dirOf
configFile>/session.sh` with the default launch line kept for the
default, and the `docs/nix.md` options table names the derived path
plus the relocated launch form (`~/.config/<that path>`).

`hmRelocated` now pins the pairing (the load-bearing test): it moved to
a *different* directory than the default (`"myscoot/custom.toml"`, so a
stuck default path would show) and sets `sessionScript`. Eval-time pins
assert the script exists beside the config *and* that nothing remains
at `"scoot/session.sh"`; a new content check (3b) pins shebang +
executable bit on the relocated script. The default half is pinned the
other way: `hmFull` keeps the default `configFile` with `sessionScript`
set, and its existing check still asserts `scoot/session.sh`.

Reconciled every other `session.sh` hit tree-wide (`grep session.sh`):
`docs/configuration.md`'s two are user-local `~/bin/` examples,
unrelated to the HM module; `nixos-session-command.md` ("writes
`scoot/session.sh`") stays accurate for defaults; the resolved
`flake-consumer`/`startup-programs` entries describe the default shape
or the generic script concept — all left alone. Interplay with #171
noted, not changed: the NixOS entry's `Exec` is a bare
`scoot --tty` referencing no script path, so this fix moves nothing on
that ticket's side.

Explicitly out of scope (separate tickets, untouched): the
session-command option (#171), CI coverage (#173), all
compositor/client code (zero `.rs` files changed).

## Evidence (record, branch `backlog/hm-session-script-path`)

Base: `main` at `4ccc7e7`. Where each command ran is stated.

- Mac (aarch64-darwin),
  `nix build .#checks.aarch64-darwin.scoot-modules --print-build-logs`:
  all 8 content checks pass, including the new `ok: session script
  follows the relocated config` (eval-time pins pass — a false one
  fails the build at eval).
- Dev VM (aarch64-linux, via the 9p mount `/mnt/scoot`, verified to
  carry the branch: `git status` there shows the same three modified
  files),
  `nix build .#checks.aarch64-linux.scoot-modules --no-link` → exit 0
  with the same 8 green checks. (`--no-link`: the 9p mount refuses the
  trailing `result` symlink with `Permission denied`; the build itself
  is green — mount limitation, not a check failure.)
- Mac `nix flake check` → exit 0 (`scoot-modules`, both `apps`,
  `formatter`; linux systems omitted — no working linux builder from
  this host, see below; x86_64-linux additionally not runnable anywhere
  here, no x86 hardware — the check is arch-independent eval plus
  python, covered by aarch64-linux).
- Linux builder (`:31022`) NOT usable: every remote derivation failed
  fetching from `cache.nixos.org` with `Problem with the SSL CA cert
  ... /etc/ssl/certs/ca-certificates.crt` — the known
  `NIX_SSL_CERT_FILE` launcher gotcha (`vm/README.md`), an environment
  fault predating this change (all failures are cache fetches, none
  reach the build; same failure the #172 ticket recorded). Not
  restarted per policy; the dev VM build above is the Linux proof.
- Rendered-path matrix (Mac eval of the module, `sessionScript` set):
  default → `[ "scoot/config.toml" "scoot/session.sh" ... ]`;
  `myscoot/custom.toml` → `[ "myscoot/custom.toml"
  "myscoot/session.sh" ... ]`; bare `config.toml` → `[ "config.toml"
  "session.sh" ... ]` (the `"."` guard biting); nested `a/b/c.toml` →
  `[ "a/b/c.toml" "a/b/session.sh" ... ]`.
- Default byte-identical proof (throwaway worktree at `main`
  `4ccc7e7`, same eval both sides, worktree removed after): old and new
  keys, script text (`#!/bin/sh\nwaybar &\nexec foot\n`) and
  executable bit all equal.
- `nixfmt --check` clean on `nix/modules/home.nix` + `nix/tests.nix`
  via the flake's own formatter build
  (`.#formatter.aarch64-darwin`). Note: bare `nix fmt` in this tree
  errors with `unexpected end of input / expecting expression` (nixfmt
  reading empty stdin — no file list passed); pre-existing quirk,
  nothing to do with this change (`git status` confirms it modified no
  files). `vm/compositor-deps.nix` is unformatted independent of this
  change — already tracked under the flake-polish ticket (gh #175).
- Benchmark: n/a — eval-time Nix change; nothing runs per-event or
  per-frame. No Rust code changed, so the cargo verification set does
  not apply (stated, not skipped silently).
