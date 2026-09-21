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
  path. Accepted cost, stated in the option description (moved there in
  the review follow-up — see below; it previously lived only in the
  source comment): a set value replaces the whole line, so dropping
  `--tty` breaks the entry loudly at the greeter, not at eval. That failure is contained — the entry
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
  verbatim when set. Guard assertions: explicitly empty or
  whitespace-only strings refused at eval (either would render an
  `Exec=` with nothing launchable that fails at the greeter).
  Existing assertions untouched (`package = null` with the
  entry on still refuses, command or not).
- `nix/modules/home.nix`: `sessionScript` description + module comment
  now name the NixOS `session.command` pairing (the full
  `<package>/bin/scoot --tty -- /home/<user>/.config/<path>` line —
  absolute, since `Exec=` gets no shell expansion).
- `docs/nix.md`: `session.command` row in the NixOS options table, the
  session prose shows the HM pairing together (append shape + wrapper
  shape + verbatim-quoting note), `sessionScript` row points back.
- `nix/tests.nix`: `osSessionCmd` (append), `osSessionWrapper`
  (`writeShellScriptBin` wrapper path — the #171 shape),
  `osSessionQuoting` (spaces/quotes/pipes verbatim), `osCmdNoSession`
  (command set, entry off → no entry); pins for assertions-hold on all
  four, refusal of empty-string, whitespace-only, and
  package-less-with-command, and
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

## Follow-up evidence (review round, uncommitted tree atop `e56275d`)

- Mac: `nix fmt -- --check nix/modules/nixos.nix nix/modules/home.nix
  nix/tests.nix` → exit 0 (one nixfmt reflow applied to the new
  assertion first). Repo-wide `nix fmt -- --check` emits a pre-existing
  `<stdin>` parse error identically with these changes stashed — not
  ours.
- Rendered Exec, doc-example shape (throwaway `/tmp/verify-exec.nix`,
  since removed; module eval with `session.command =
  "<fakePkg>/bin/scoot --tty -- /home/alice/.config/scoot/session.sh"`):
  `Exec=/nix/store/g2q20z3yd3yghgvklqgg1vqsfbsy2cy5-fake-scoot/bin/scoot
  --tty -- /home/alice/.config/scoot/session.sh` — absolute, no `~`,
  assertions hold. Same harness with the OLD `~` value renders
  `Exec=... -- ~/.config/scoot/session.sh` with the tilde literal
  (verbatim, nothing expands it) — the reading-only proof the old
  example was broken.
- Mac: `nix flake check --print-build-logs` → darwin green;
  `nix build .#checks.aarch64-darwin.scoot-modules --print-build-logs
  --rebuild` → all 12 content lines green (incl. verbatim quoting).
- Dev VM: same check rebuilt (`--rebuild`) → all 12 lines green.
- Mac: `nix eval .#checks.x86_64-linux.scoot-modules.drvPath` →
  `"...-scoot-modules-check.drv"`, clean — the new whitespace-only
  refusal pin evaluates (a broken guard would fail eval, not the build).
- Zero `.rs` touched — cargo suite n/a, same as the original round.

## Review follow-up (blocking doc finding + two notes, same branch)

The review of `e56275d` came back DO NOT MERGE on one blocking finding:
every `Exec=`-context example used `~/.config/scoot/session.sh`, which
is broken as written — the Desktop Entry Spec provides no tilde
expansion (unquote → field codes → argv, no shell step; `~` is
reserved), and scoot's `State::spawn` is `Command::new(program)`
directly with no shell/tilde logic. Followed verbatim, the example
yields ENOENT → fail-open warn-and-continue → a bare session: the exact
symptom #171 was filed to eliminate. Fixed doc-only: all
`Exec=`-context examples now use an absolute path
(`/home/alice/...` in `docs/nix.md` + the `nixos.nix` example,
`/home/<user>/...` in `home.nix` where the tail varies with
`configFile`), and one sentence where the verbatim note lives states
`Exec=` lines get no shell expansion. The shell-invocation
`Launch it with scoot -- ~/.config/...` lines are untouched — correct
for shells, out of scope. The old example was proven broken by reading
only (spec + spawn path, per the review); no greeter was staged.

Non-blocking notes, folded in: (1) the dropping-`--tty` cost sentence
now lives in the option `description` itself, not just the source
comment (the record's earlier "stated in the option description" now
holds); (2) the empty-string eval guard was extended to
whitespace-only (`builtins.match "^[[:space:]]*$" ... == null` — null
on no match, so the disjunct is false exactly for empty/blank), pinned
by a new refusal pin in `nix/tests.nix`. The call: extending was a
three-line change on the same assertion, and `" "` renders an `Exec=`
of blanks that fails at the greeter exactly like `""` — leaving it
would keep a known-broken value eval-clean.

## Left out, with why

- `README.md` untouched: it documents no Nix module options (that
  reference lives in `docs/nix.md` + the option descriptions), same as
  the preceding flake-only tickets.
- No `session.command` argv-list variant: one type, verbatim, keeps the
  wrapper shape expressible (see decision).
- CI changes (#173) and gpu-scanout (#177): explicitly out of scope.
