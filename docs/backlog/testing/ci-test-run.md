---
title: "No CI: every verification run is manual and self-reported — LANDED 2026-09-19 (PR #140), except the `--features gpu-scanout` build, which is written and gated until PR #135 puts that feature on `main`"
status: "resolved"
area: "testing"
priority: "high"
blocked: null
---

# No CI: every verification run is manual and self-reported — LANDED 2026-09-19 (PR #140)

**What landed is at the bottom of this file** ("What landed", below). The
entry is kept at this path rather than moved to `resolved/`, against the
usual convention, because `CLAUDE.md` links `docs/backlog/testing/ci-test-run.md`
by name and an implementer may not edit `CLAUDE.md`.

Requested 2026-09-19. There is **no `.github/workflows` at all** — verified
during the 2026-09-17 audit of PRs #73–#89 and still true. Every number in
every PR body is a self-reported run on the dev VM, and the only thing
catching a wrong one is that the reviewer and the coordinating session
re-derive it by hand.

That process has actually worked — it caught a falsified hardware record, a
stale evidence SHA, and a test-count claim that belonged to a different PR.
But it works because humans and agents keep choosing to do it, and nothing
enforces it. CI is the floor under that, not a replacement for it.

## Why it matters more now

`CLAUDE.md` now leans on nextest as the required runner, and nextest
structurally **cannot** see cross-test interference — it isolates every
test. `cargo test` is the only thing that catches a test which breaks when
sharing a process, and it is no longer a required local step. CI is where
that coverage is supposed to live. Until this exists, that coverage is
simply not being taken.

## What it should run

- `cargo fmt --check -p scoot`
- `cargo clippy -p scoot --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- **`cargo test`** — specifically for the interference coverage above, not
  as a duplicate of nextest
- `scripts/smoke-test.sh` under `--headless` (it is backend-agnostic and
  needs no seat)
- a build with `--features gpu-scanout` as well as without, since the
  GPU-free default build not linking libgbm is a **hard requirement**, not a
  preference — `ldd` asserting no `libgbm`/`libEGL` in the default build is
  a cheap, high-value CI check that no local habit enforces

What it cannot run: anything needing a real DRM/VT seat (`--tty`), and
anything needing a GPU. Those stay manual on the dev VM and on Asahi
hardware, and the workflow should say so rather than implying coverage it
does not have.

## Design notes

**Use the flake, not hand-listed apt packages.** `README.md` states the
flake is the only dependency set this repo maintains; a CI job installing
libseat/libinput/libxkbcommon/pixman/udev by hand would be a second
dependency list that silently drifts from the first. `nix develop -c ...`
keeps one source of truth.

**Caching is the whole cost question.** Smithay is a large dependency tree
and this is a debug build; without a cargo cache a run is many minutes. The
dev VM's shared `/var/cargo-target` is why local runs feel instant, and CI
has no equivalent unless one is configured.

**macOS matters more than it looks.** `scoot` compiles out the compositor
on macOS and ships only the `msg` client, and that path has broken before
without anyone noticing locally, because nobody builds it on the Mac between
releases. A cheap `cargo check` job on macOS would cover it.

## One caution

The existing per-PR process is *stronger* than typical CI — an independent
reviewer plus a coordinator re-deriving the numbers catches classes of
problem no workflow will. The risk to watch for is CI becoming a reason to
skip that: a green check is evidence the tests passed, not evidence the
change is right. `CLAUDE.md`'s review gate is unchanged by this item.

## What landed (PR #140, 2026-09-19)

`.github/workflows/ci.yml`. Two jobs, on every `pull_request` and every push
to `main`; every step runs through `nix develop`, so the flake stays the one
dependency list.

**Linux (`ubuntu-latest`)**

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
  -D warnings`. Supersets of the documented `-p scoot` steps: `scoot-core`
  and `scoot-ipc` cost nothing extra once their dependencies are built here,
  and nothing else lints them. Both were verified clean at `64f5fea` before
  widening.
- `cargo build -p scoot`, then **`ldd` asserting the default binary links no
  `libgbm` and no `libEGL`**, matching the *soname* field so an unrelated
  store path containing the string cannot trip it, and covering the whole
  `DT_NEEDED` closure so a GPU library arriving transitively is caught too.
  Then `cargo build -p scoot --features gpu-scanout` and the mirror
  assertion that *that* binary does link `libgbm` — without the positive
  control, an `ldd` aimed at the wrong binary would leave the no-GPU check
  passing while testing nothing.
- `cargo nextest run --workspace`.
- `cargo test --workspace`, carrying a comment in the file saying why it is
  not a duplicate of the line above it, because someone will try to delete
  it as one.
- `scripts/smoke-test.sh` under `--headless`. Its harness tools (`foot`,
  `jq`, ImageMagick, `wayland-info`) come from `nix shell --inputs-from .`,
  pinned to this repo's own `flake.lock` rather than a second floating
  nixpkgs; `SMOKE_PREFIX` keeps every socket out of `/run/user/<uid>`, which
  a runner does not have.

**macOS (`macos-latest`, arm64)** — `cargo check --workspace --all-targets`,
the `scoot msg` half nobody builds between releases.

### Evidence

Run <https://github.com/scoot-sh/scoot/actions/runs/35454377003> (commit
`44f7d3e`, both jobs green):

- `cargo nextest run --workspace` — `1100 tests run: 1100 passed, 3 skipped`
- `cargo test --workspace` — `995 passed; 0 failed; 3 ignored` for the
  `scoot` binary, matching the local baseline exactly
- `ok: the default build links no libgbm and no libEGL`
- `ok: --features gpu-scanout links libgbm, so the default build's
  assertion is a real one`
- smoke test: every `ok:` line, including the three decoration pixel checks
  and `zwp_linux_dmabuf_v1 is advertised`

Wall clock, Linux job: **8m05s cold** (empty cargo cache, run 35453015885)
against **4m30s warm** (run 35454377003) — clippy 2m09s → 11s, build 2m09s
→ 12s, nextest 2m29s → 53s. macOS: 3m46s cold, 1m33s warm. The cargo cache
restores in ~1m (1.4 GB) and is saved with `if: always()`, so a red run's
artifacts survive for the next attempt.

### Two findings from the first runs

1. **`render/tests.rs`'s GLES tests fail on a machine with no EGL at all**,
   which the dev VM never is. Run 35453015885 died at
   `a_capture_covers_the_whole_target_at_the_backends_own_size` with
   `Failed to load LibEGL: libEGL.so.1: cannot open shared object file`,
   taking 333 unrun tests with it. That module says failing rather than
   skipping is deliberate, on the reasoning that any machine able to run
   the suite has Mesa's software EGL — the dev VM resolves the `dlopen`
   through `/run/current-system/sw/lib`, which is in the binary's `RUNPATH`
   there and does not exist on a runner. Handled by giving CI the software
   EGL the suite documents needing (Mesa through `--inputs-from .`), scoped
   to the two test steps by name, so the smoke test still runs with no
   loadable EGL at all.
2. **`magic-nix-cache-action@v15` now requires a FlakeHub account.** It
   failed to authenticate on every run, cached nothing usable, cost 11s up
   front and up to 1m51s in its own post step on macOS, and left ~780 small
   cache entries behind. Removed; the dev shell comes from cache.nixos.org
   in ~35s of the first `nix develop`. The cargo cache is where the time
   actually is.

### Still not covered, by construction

`--tty` and everything a DRM/VT seat implies, GPU *hardware* paths (the
GLES tests run against Mesa's software rasteriser, never a driver; no run
starts a live compositor with `--renderer gles`), `--nested`, and
performance. The workflow's header block says so at the top of the file
rather than leaving a green check to imply otherwise.
