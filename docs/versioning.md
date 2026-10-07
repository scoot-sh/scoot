# Versioning: what a version means and how it moves

Each shipped binary carries its own `version` in its crate's `Cargo.toml`
and prints it from `--version`. There are two version lines, not four:
the tightly coupled trio `scoot`/`scootctl`/`scootbg` moves in lockstep,
and `scootbar` moves on its own. Internal libraries (`scoot-core`,
`scoot-ipc`, `scootbg-mem`) are `publish = false` and version with their
consumer. `scripts/version` enforces the policy (`check`), computes the
next bump from history (`plan`, a dry run), and writes release notes
(`changelog`). Tags name exactly one package: `scoot-v0.2.0`,
`scootbar-v0.3.0`.

Note on names: there is no `scootctl` binary. `scootctl` is the client
library behind `scoot msg`, and it versions inside the trio because the
`scoot` binary embeds it: a client change is a compositor-binary change.
The ticket's "shipped `scootctl`" is that library.

## What SemVer means for each binary

A version names the binary's **public interface**: its CLI flags, its
config-file schema, and its control-socket protocol. A change that forces
a user or an integrating agent to do something different is a bump; an
internal change that changes no observable behavior is not.

| Binary | Public interface | What breaks it (major) |
|---|---|---|
| `scoot` | CLI flags (`--headless`, `msg` grammar, `--version` line shape); the config-file schema (`[layout]`, `[binds]`, every key); the `scoot msg` request/action/event grammar and reply shapes; the IPC protocol number | Removing or renaming a flag, a config key, or a request/action; changing a reply field's meaning; any IPC protocol bump that old clients cannot speak |
| `scoot msg` (via `scootctl`) | The request/action grammar it parses and the help/JSON it renders | Same grammar breaks as `scoot` (it is the same grammar, one parser) |
| `scootbg` | Commands and flags (`daemon`, `set`, `apply-config`, `--profile`); the `[wallpaper]` section schema it accepts; the `apply-config` handshake below | Removing a command or flag; refusing a section shape an older release accepted |
| `scootbar` | Flags and subcommands (`daemon`, `msg`, `--check`); the bar config-file schema; the `scootbar msg` agent interface | Same shape of break, scoped to the bar |

`feat` is a minor bump (a new flag, a new config key, a new request),
`fix`/`perf` a patch one. `refactor`/`test`/`build`/`chore`/`docs`/`ci`
with a package scope never bump: housekeeping, no behavior change -- if
behavior changed it should have been a `feat`/`fix`. Pre-1.0 (today):
a major bump lands as a minor one (`0.1.0` -> `0.2.0`, never `1.0.0`
undeclared); the maintainer declares 1.0, and past it the normal rules
apply. Config-schema strictness is load-bearing here: `scootbg` refuses
unknown section keys (`deny_unknown_fields`), so a newer `scoot` sending
a key an older `scootbg` does not know fails loudly (exit 2, the session
carries on with its background color) rather than silently dropping it.

## The couplings, measured

Independent versions make the couplings visible, so each is an explicit
contract, enforced where a mismatch would misbehave:

- **`scoot` / `scoot msg`: the IPC protocol number** (`scoot-ipc`'s
  `PROTOCOL_VERSION`, today 10). `--version` prints it beside the
  binary's version (`scoot 0.1.0 (ipc protocol 10)`), and the `version`
  reply carries both, so a client checks compatibility before
  connecting. Same grammar, one parser (`scootctl`), pinned by
  containment tests on both sides.
- **`scoot` / `scootbg`: the `apply-config` handshake** (already in the
  code, `crates/scootbg/src/apply.rs`). Every run opens with a
  `version` request on the same connection: a different protocol is an
  error, a different binary version a warning, and a daemon too old to
  know `apply-config` at all is refused with the fix (`scootbg kill`,
  then run again -- `scoot` does on reload). scoot never waits on the
  run (it would deadlock: the reply needs this compositor processing
  commits), and a failed run keeps the session -- wallpaper included --
  alive and retries on the next reload. Measured against the code and
  the existing `apply` tests: a version mismatch warns and still
  applies; a protocol mismatch refuses loudly; a missing daemon is
  started. No new handshake was needed, so none was added.
  Compatibility rule: **same protocol required (enforced), version
  mismatch tolerated with a warning**. The trio's lockstep makes even
  the warning rare: both halves ship together.
- **`scootbar` / `scoot`: standard protocols only**, plus the optional
  IPC feature guarded by the IPC protocol number. A bar from one
  release works against a compositor from another as long as both speak
  the same Wayland protocols; the IPC-guarded option refuses cleanly
  past a protocol bump. This is where independence pays: the bar
  releases without the compositor and the other way round.

## The tool

`scripts/version` (stdlib only, like `scripts/backlog`) reads history
and manifests and writes nothing, creates nothing, pushes nothing:

```sh
scripts/version plan              # dry run: bump per package, tags it would create
scripts/version plan --verbose    # ...plus the commits behind each bump
scripts/version check             # fail CI when manifests disagree with the policy
scripts/version changelog scootbar  # release-notes markdown since the last tag
```

The bump per package comes from the commits since its last
`<package>-vX.Y.Z` tag (the whole history when there is no tag yet)
that name it: `feat` minor, `fix`/`perf` patch, `!` or
`BREAKING CHANGE:` major. A commit names packages in its scope
(`fix(scoot,scootbar)` moves both); internal-library scopes fan out to
their consumers, the same fan-out CI uses (`scoot-ipc` moves all four,
`scoot-core` the trio). Commits with no package scope never bump:
unscoped subjects, `ci`/`docs`/`nix`/`backlog`/`claude`/`deps` scopes,
and merge subjects. Squash-merges land the PR title as the subject, so
they parse like any other commit (PR titles follow the same form).

`check` (what CI runs) verifies the declared policy, not the history:
valid `X.Y.Z` semver everywhere, the trio equal, each shipped crate
pinned to its own `version` (never `version.workspace`), each internal
library `publish = false`, and no tag ahead of its manifest (with
`--allow-ahead` for the release commit the tag is about to point at).

Tests live in `scripts/test_version.py` (fixture histories: expected
bumps, no-bump cases, breaking cases, merge/squash subjects, unscoped
commits, no prior tag) and run in CI beside the check.

## Cutting a release (maintainer)

The tool never tags; the maintainer does the first release by hand:

1. `scripts/version plan --verbose` on main, and read what it would do.
2. Bump each moving crate's `version` in its `Cargo.toml` to the
   planned number (the trio together, the bar on its own).
3. Commit (`chore(scoot,scootbg): release 0.2.0` -- a non-package scope
   pairing never bumps, so the release commit itself moves nothing),
   then tag each moving package (`scoot-v0.2.0`, `scootbg-v0.2.0`, ...).
4. Push the commit and the tags. The tag-triggered release workflow
   (blocked on this ticket) takes it from there: per-package changelog
   from `scripts/version changelog <package>`, vendored tarball,
   checksums, signatures.
