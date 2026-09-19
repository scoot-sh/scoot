---
title: "No CI: every verification run is manual and self-reported"
status: "open"
area: "testing"
priority: "high"
blocked: null
---

# No CI: every verification run is manual and self-reported

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
