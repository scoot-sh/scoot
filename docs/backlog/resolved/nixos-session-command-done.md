---
title: "NixOS module: session entry has no way to launch a shell (Exec is a fixed `scoot --tty`)"
status: "resolved"
area: "packaging"
priority: "medium"
blocked: null
---

# NixOS module: session entry has no way to launch a shell (Exec is a fixed `scoot --tty`) — DONE

Filed as gh issue #171. `nix/modules/nixos.nix` built the login entry
with a fixed `Exec=${cfg.package}/bin/scoot --tty`, so the greeter
started a bare compositor while the home-manager module's
`scoot/session.sh` sat written, executable, and ignored.

## Decision (recorded per the ticket)

**(a) Full-`Exec=` override, as a plain string** — `session.command :
nullOr str`, default `null`.

- *Override vs append (a over b):* the issue's acceptance shape is a
  WRAPPER (`Exec=<wrapper>/bin/scoot-session`, which itself runs
  `scoot --tty -- ...` plus stderr to a log). A `-- COMMAND` append
  cannot express that — it would nest scoot inside scoot. A verbatim
  full line expresses all three shapes: bare default (null), the common
  append (user writes the full `scoot --tty -- ...` line), and a wrapper
  path. Accepted cost, stated in the option description: a set value
  replaces the whole line, so dropping `--tty` breaks the entry loudly
  at the greeter, not at eval. That failure is contained — the entry
  stays additive and default-off whatever the value, so the other
  sessions remain and nobody is stranded.
- *String vs argv list:* an `Exec=` line is a command line, not argv;
  auto-escaping would fight wrapper paths, and per the Desktop Entry
  Spec the quoting is the author's anyway. Verbatim puts quoting
  visibly in the user's hands with a doc example; the content check
  pins spaces/quotes/pipes/redirection surviving intact.

## What changed

- `nix/modules/nixos.nix`: new `session.command` option (null =
  today's bare `<package>/bin/scoot --tty`); `Exec=` renders it
  verbatim when set. One guard assertion: explicitly empty string
  refused at eval (would render an empty `Exec=` that fails at the
  greeter). Existing assertions untouched (`package = null` with the
  entry on still refuses, command or not).
- `nix/modules/home.nix`: `sessionScript` description + module comment
  now name the NixOS `session.command` pairing (the full
  `<package>/bin/scoot --tty -- ~/.config/<path>` line).
- `docs/nix.md`: `session.command` row in the NixOS options table, the
  session prose shows the HM pairing together (append shape + wrapper
  shape + verbatim-quoting note), `sessionScript` row points back.
- `nix/tests.nix`: `osSessionCmd` (append), `osSessionWrapper`
  (`writeShellScriptBin` wrapper path — the #171 shape),
  `osSessionQuoting` (spaces/quotes/pipes verbatim), `osCmdNoSession`
  (command set, entry off → no entry); pins for assertions-hold on all
  four, refusal of empty-string and package-less-with-command, and
  `defaultSession == null` for the command/wrapper shapes
  (never-strand); content checks 6b–6e.
- Zero `.rs` touched (`git diff --stat`: `docs/nix.md`,
  `nix/modules/home.nix`, `nix/modules/nixos.nix`, `nix/tests.nix`
  only) — cargo suite not run, nothing to run it against. Benchmark
  n/a: eval-time option, no hot path.

## Evidence (recorded, not narrated)

Branch `backlog/nixos-session-command`, from `main` at `b737f71`.

- Mac (`/Users/steveyackey/code/flexwm`):
  `nix fmt -- --check nix/modules/nixos.nix nix/modules/home.nix nix/tests.nix`
  → exit 0.
- Mac: `nix flake check --print-build-logs` → all 5 checks green,
  incl. `checks.aarch64-darwin.scoot-modules` with:
  `ok: session.command appends the session command to Exec`,
  `ok: session.command expresses the wrapper-script entry`,
  `Exec=/nix/store/g2q20z3yd3yghgvklqgg1vqsfbsy2cy5-fake-scoot/bin/scoot --tty -- sh -c 'exec foot 2>/tmp/scoot.log | cat'`
  + `ok: ... renders verbatim`, `scoot-modules: all file-content checks passed`.
- Raw rendered entries (built store artifacts, darwin):
  default `Exec=/nix/store/g2q20z3yd3yghgvklqgg1vqsfbsy2cy5-fake-scoot/bin/scoot --tty`;
  append `Exec=...-fake-scoot/bin/scoot --tty -- ...-fake-scoot/bin/my-shell`;
  wrapper `Exec=/nix/store/bi2n65fxg55sj8vp3mjinb8av1pp8rgb-scoot-session/bin/scoot-session`.
- Byte-identical default proven by store path: HEAD's
  `checks.aarch64-darwin.scoot-modules` (built from a throwaway
  `git worktree` at `b737f71`, since removed) embeds the default entry
  at `/nix/store/llc45yjifnlxd70xrrh680hki812paw7-scoot-wayland-session`
  — the same path this branch renders (input-addressed: same nixpkgs,
  same template, same interpolation ⇒ same bytes).
- Dev VM (`ssh -p 2222 dev@localhost`, repo at `/mnt/scoot`, native
  aarch64-linux): `nix ... build /mnt/scoot#checks.aarch64-linux.scoot-modules
  --print-build-logs` → all 12 content lines green, same shapes.
- Mac: `nix eval .#checks.x86_64-linux.scoot-modules.drvPath` →
  `"...-scoot-modules-check.drv"`, clean (all eval-time `_pins`
  asserts hold, incl. the new refusal/additive pins). The x86_64-linux
  *build* was not runnable: `nix flake check --all-systems` routes it
  to the `linux-builder` VM, which fails every download with the known
  `NIX_SSL_CERT_FILE` gotcha (`vm/README.md`: the builder was launched
  from a shell without the export, so no CA bundle was shared in) —
  environmental, unrelated to this change; VM lifecycle left alone per
  policy. aarch64-linux green + x86_64-linux eval green is the full
  runnable coverage.

## Left out, with why

- `README.md` untouched: it documents no Nix module options (that
  reference lives in `docs/nix.md` + the option descriptions), same as
  the preceding flake-only tickets.
- No `session.command` argv-list variant: one type, verbatim, keeps the
  wrapper shape expressible (see decision).
- CI changes (#173) and gpu-scanout (#177): explicitly out of scope.
