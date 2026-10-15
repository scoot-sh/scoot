# Versioning: one version for the whole repository

scoot has one version. `[workspace.package] version` in the root
`Cargo.toml` is it, and every crate inherits it with
`version.workspace = true`: the binaries (`scoot`, which carries
`scoot msg`; `scootbg`; `scootbar`; later `scootnotify`, `scootlaunch`,
`scootgreet`) and the libraries (`scootctl`, `scoot-core`, `scoot-ipc`,
`scootbg-mem`). A release is one tag, `vX.Y.Z`, that releases every binary
at that version. There are no per-package versions or tags, and nothing is
published to crates.io (`publish = false` stays where it is).

This replaced per-package versions (a lockstep scoot/scootctl/scootbg trio,
an independent scootbar, `scoot-vX`/`scootbar-vX` tags and the
`scripts/version` tool) before any of them were released. Those tags never
existed, so nothing has to migrate.

## What a version means

A version names the **whole suite's public interface**: every binary's CLI
flags, every config-file schema (`[layout]`, `[binds]`, `[wallpaper]`, the
bar's config), the `scoot msg` request/action/event grammar and reply shapes,
and the control protocols below. A change that makes a user or an
integrating agent do something different is a bump for the whole suite,
whichever binary it touched.

Inside one release: **same version**. Install, upgrade
and run every binary at one version. Across versions, compatibility is
carried by protocol numbers, never by crate versions.

## The protocol numbers

Each wire contract between two processes has its own number. A number only
moves when an old peer would misread a new one. Versions say nothing about
compatibility; these numbers do.

| Contract | Constant (today) | Between | Enforced |
|---|---|---|---|
| scoot IPC | `scoot_ipc::PROTOCOL_VERSION` (11) | scoot and any IPC client: `scoot msg` from another build (a Mac client driving a VM), agents, scripts | Advertised, not enforced. `scoot --version` prints it (`scoot 0.2.1 (ipc protocol 11)`) and the `version` reply carries it. A client compares it before relying on newer requests. `scoot msg` itself does not refuse a mismatch yet. |
| scootbg control | `scootbg::protocol::PROTOCOL_VERSION` (1) | scoot (`[wallpaper]` through `scootbg apply-config`) or the `scootbg` CLI, and the scootbg daemon | Yes, both ends. Every request names its protocol and the daemon refuses another one. `apply-config` opens with a `version` request: a different protocol is an error with the fix (`scootbg kill`, then reload), a different version only a warning. |
| scootbar control | `scootbar::control::protocol::PROTOCOL_VERSION` (1) | `scootbar msg` and the scootbar daemon | Yes. A request whose protocol differs is refused. |
| Looks `schema` | not landed yet | the looks a desktop profile reads | To be numbered and checked when it lands. |

scootbar talks to the compositor through standard Wayland protocols. Its only
scoot-specific call is the `{ scoot = "quit" }` action, one fixed IPC line
pinned by a test against `scoot-ipc`.

New request types and fields can stay inside a protocol number when an older
peer answers them safely. For example, an old scootbg answers
`set-workspace` with `unknown request`. Config-schema strictness is what makes
this safe: scootbg refuses unknown section keys (`deny_unknown_fields`), so a
newer scoot sending a key an older scootbg does not know fails loudly (exit 2,
the session keeps its background color) instead of silently dropping it.

The look-scoped reload follows the same rule, with the tag doing the work.
`reload_look` (`scoot msg reload --look`) is a distinct request tag, not a
`scope` field on `reload`: a `scope` field would be unsafe here because
serde's internally-tagged unit variants drain and ignore extra fields, so an
older compositor would decode `{"type":"reload","scope":"look"}` as a plain
reload and apply the pending edits the scope exists to skip. With a distinct
tag an older compositor answers an ordinary unknown-request `error` and keeps
serving -- never a plain reload. The client prints that error and exits
non-zero, with the running config untouched. An additive new request like
this is not breaking, so `PROTOCOL_VERSION` stays 11 and the commit carries
no `!` marker.

## How a version moves

Conventional Commits feed the changelog and propose the bump; the maintainer
decides it in the release PR.

- **Pre-1.0 (today)**: if any commit since the last tag is breaking (`!`
  after the type or scope, or a `BREAKING CHANGE:` footer), the next version
  bumps the minor (`0.2.3` -> `0.3.0`). Otherwise it bumps the patch
  (`0.2.3` -> `0.2.4`), `feat` included.
- **1.0** is the maintainer's call. It comes once the IPC protocol and the
  config schemas have a written support promise. After that the usual SemVer
  rules apply: breaking bumps the major, `feat` the minor, everything else
  the patch.
- "Breaking" means the public interface above: a removed or renamed flag,
  config key or request, a reply field that changes meaning, or a protocol
  bump an older peer cannot speak.
- A commit counts only if it touches a crate under `crates/`. A change only to
  `resources/`, `packaging/`, the flake or the site neither moves the version
  nor appears in the generated changelog. Write a hand note under the
  Unreleased heading in `CHANGELOG.md` when one of those matters to users.

## The release PR

[release-plz](https://release-plz.dev) (`release-plz.toml`,
`.github/workflows/release-plz.yml`) keeps one pull request open on main,
titled `chore: release vX.Y.Z`. It:

1. bumps `[workspace.package] version` (and the workspace entries in
   `Cargo.lock`), then commits `scripts/sync-fuzz-locks` to the branch so the
   fuzz crates' own lockfiles (`crates/*/fuzz/Cargo.lock`, not in the
   workspace) follow;
2. inserts `## [X.Y.Z] - <date>` directly under `## Unreleased` in the root
   `CHANGELOG.md`, with a list generated from the commits since the last tag,
   grouped by scope (component) and labeled by kind. The hand-written notes
   that were under Unreleased now sit inside that version's section.

A new crate under `crates/` needs a `[[package]]` entry in `release-plz.toml`
with its own `git_tag_name` (for example `<crate>-v{{ version }}`) the first
release after it lands. All packages share the `v{{ version }}` tag, so
without it release-plz looks for the crate in the previous release's checkout,
where it does not exist, and the release PR job fails. `scoot-look` is the
worked example; the crate also goes in the `scoot` package's
`changelog_include`.

The maintainer reviews the PR and may change the version in `Cargo.toml` and
the heading in `CHANGELOG.md` before merging. Merging it is the release:
release-plz creates the `vX.Y.Z` tag and then starts
`.github/workflows/release.yml` on that tag. That workflow checks the tag
(`scripts/release-resolve`), builds every binary, and publishes the one
GitHub Release with the vendored source tarball, portable binaries for both
architectures, an SBOM, checksums, provenance, and the version's
`CHANGELOG.md` section as its notes (`scripts/release-notes`).

**No secret is involved.** release-plz runs on the workflow's own
`GITHUB_TOKEN`. GitHub starts no workflow for an event that token causes,
so neither the tag nor the PR would start anything on their own. The one
documented exception is that "workflow_dispatch and repository_dispatch
events always create workflow runs" ([Triggering a workflow from a
workflow](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow)).
So `release-plz.yml` dispatches the next step itself:

- After creating the tag, it runs `release.yml` on it (`--ref vX.Y.Z`,
  `dry_run=false`).
- After opening or updating the release PR, it runs the PR's checks on
  the PR's branch: `ci.yml`, `packaging.yml` (vendoring, licenses,
  advisories) and `release.yml` with no `tag`, which runs only its lint job
  (actionlint, the guards, the release tool tests). A dispatched run's
  check suite attaches to the branch's head commit, which is the PR's head,
  and GitHub associates it with the PR.
  This was measured on #586: the run's `pull_requests` listed the PR. The
  PR's own `pull_request` runs still appear, held until a maintainer presses
  "Approve workflows to run".
  - **One caveat, also measured on #586.** When a `pull_request` run of
    `ci.yml` exists for the same commit, the PR's check list (the merge box,
    `gh pr checks`) shows one entry per check name, and it shows the
    `pull_request` one. The dispatched results are then only on the commit's
    checks and in the Actions tab.
  - **On a release PR, the held runs have not started, so they have no check
    runs of those names.** The dispatched ones should then fill the list.
    That has not been observed yet: the first release PR will show it.
  - Approving the held runs is optional; they repeat the same jobs.

One repository setting is needed for GITHUB_TOKEN to open the PR at all:
Settings > Actions > General > Workflow permissions > "Allow GitHub Actions to
create and approve pull requests". It also lets workflows approve pull
requests. Nothing here approves one, and main has no required reviews.

A maintainer can still push a `v*` tag by hand, which starts `release.yml`
through its `push` trigger. Both paths share one concurrency group per tag,
and `release-resolve --complete-ok` turns an already-complete release into a
green no-op. A tag pushed by hand and dispatched by release-plz therefore
releases once. A dispatch that would publish must run on the tag itself:
from a branch it is refused before anything builds, and only a dry run is
allowed there.

The release PR is also where the distro packaging moves: bump `pkgver`
in the Arch release PKGBUILDs (with `.SRCINFO`) and `Version` in the RPM
specs to the proposed version before merging. The Arch, Debian and RPM
workflows are manual-only (`workflow_dispatch`): run each by hand before
shipping a package, and their gates fail when the packaging lags
the workspace version. Checksums are filled in once the tag's tarball
exists. The site's
[cut a release](https://www.scoot.sh/reference/cut-a-release/) page has the
steps.

## The first tag was v0.2.0, cut by hand

FlakeHub already carries rolling releases numbered `0.1.<commit count>`
(`0.1.1861` and up). A `v0.1.x` tag would have sorted below them, so the first tag
was **v0.2.0**, cut by hand on 2026-10-14, and the workspace now reads `0.2.1`
(`CHANGELOG.md` shows `[0.2.0]` and `[0.2.1]`, both dated 2026-10-14).

release-plz waits for a first `v*` tag. With none it would treat the whole
history as one first release, so the workflow skipped until that tag existed. The
maintainer cut v0.2.0 by hand (the cut-a-release page has the commands): a
small PR that added `## [0.2.0] - <date>` under the Unreleased heading, then
an annotated `v0.2.0` tag on its merge commit, pushed. From then on
release-plz proposes every release.

## What a build that is not a release calls itself

A build of any commit other than a release tag labels its version with
SemVer build metadata: `<last tag>+<commits since it>.g<commit>`.

```console
$ scoot --version
scoot 0.2.1+14.g615608c (ipc protocol 11)
```

The line keeps its shape (`scoot X (ipc protocol N)`); only `X` grows the
suffix, and the IPC `version` reply carries the same string. Each binary's
build script works it out from `git describe` (`crates/scoot-ipc/build_label.rs`):

- At a `vX.Y.Z` tag, or from a tree with no `.git` (the release tarball, a
  Nix build), the plain version: `0.2.1`.
- Before the first tag exists: the workspace version and every commit,
  `0.2.0+1863.g5159f75`.
- A tree unpacked inside some other repository (an AUR clone) gets the plain
  version, never that repository's history.
- Uncommitted changes are not marked. Cargo cannot cheaply tell a build
  script that the work tree changed, so a `.dirty` suffix would go stale.

Nix builds deliberately print the plain version. Putting `self.rev` into a
derivation would change it on every commit, so every merge (docs-only ones
included) would rebuild every package, and a `github:` user at a docs-only
commit would miss Cachix and build locally. FlakeHub's version already names
the commit (below).

Packages built from main use the same numbers in their own spelling:

| Where | Spelling |
|---|---|
| `--version` (cargo builds from a checkout, `-git` packages) | `0.2.1+14.g615608c` |
| Arch `-git` `pkgver` | `0.2.1.r14.g615608c` |
| FlakeHub rolling release | `0.3.<commits on main>+rev-<commit>` |

## FlakeHub's rolling numbers

Every push to main publishes the flake to FlakeHub as a rolling release
numbered `<major>.<minor + 1>.<commits on main>`, from the workspace version
(`scripts/workspace-version --rolling`, passed as `rolling-major` and
`rolling-minor` in `.github/workflows/nix-build.yml`). While the workspace reads 0.2.x, main publishes
`0.3.<count>`. That sorts above every `v0.2.x` tag and above the `0.1.<count>`
releases published before this scheme, so a FlakeHub constraint never prefers
an older main.

## The tools

```sh
scripts/workspace-version            # 0.2.1
scripts/workspace-version --rolling  # 0.3, the FlakeHub rolling prefix
scripts/workspace-version --check    # fails if a crate sets its own version
scripts/release-resolve v0.2.1 --no-github   # the release gate, run locally
scripts/release-notes 0.2.1          # that version's CHANGELOG.md section
```

All are stdlib Python with tests in `scripts/test_release_resolve.py`, which
`release.yml`'s lint job runs on every pull request that touches the release
tooling.
