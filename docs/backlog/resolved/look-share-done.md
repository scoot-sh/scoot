---
title: "Install looks from git (flake lookPacks split out)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "later"
resolved: "2026-11-06"
---

# Install looks from git (flake lookPacks split out)

Filed 2026-10-12, child of [`live-looks`](live-looks.md). **Later** (D12):
filed so the design has a home, not scheduled. Serves **daily-drive**.

## What to do (when scheduled)

- `scoot look install GIT-URL`: `git clone --depth 1
  --no-recurse-submodules -- URL`, `https://` and `ssh://` only, then the
  `install PATH` checks.
- A flake output `lookPacks.<name>` (a directory of looks usable as a flake
  input in `programs.scoot.looks`).

## Resolution (2026-11-06)

Half shipped, half split, on purpose:

- **`scoot look install GIT-URL` shipped** (`crates/scootctl/src/look/install/git.rs`,
  `git/url.rs`; reference in `site/src/content/docs/looks/index.md`, "Install
  from git"). The design decisions the ticket left open:
  - **URL or path** by text alone, never the filesystem: `://` means URL
    (`https`/`ssh` only), a `:` before the first `/` means a refused git
    shorthand (`host:path`, `ext::`), the rest is a path.
  - **Read, not checked out.** A bare depth-1 clone, then `git ls-tree` and
    `git cat-file blob`, so filters, LFS, `.gitattributes` conversion,
    hooks, symlinks and submodules never materialize; `.git` is never
    part of the look. The tree listing is gated (symlink, gitlink, mode,
    path, count, size) before any blob is read, then the existing
    `finish_install` runs unchanged, so `--trust` means what it does for a
    directory.
  - **The repository root is the look.** A `.gitignore` or `.github/`
    there refuses by name, as in a directory. No subdirectory flag, no
    branch or tag (the default branch's tip installs).
  - **No credentials:** environment cleared to an allowlist, system and
    global git config off, no credential helper, `https://` takes no user
    name, `ssh://` no password, prompts off, ssh in `BatchMode`.
  - **`git` is found on `PATH`** (like `curl` for wallpaper URLs); missing
    is a clear error naming the directory install as the way around it.
  - **Bounds:** 120 s, 256 MiB of temp directory watched while git runs.
  - **Known gap:** Ctrl-C leaves the 0700 temp directory (no signal handler
    in `scootctl`; every other exit removes it).
- **`lookPacks.<name>` split out** to
  [`look-packs-flake`](look-packs-flake.md): `toEntry` takes one look
  directory per entry, a pack needs a new expansion branch plus
  `nix/tests.nix` cases and a NixOS evaluation to verify, which is its own
  change.

## Not in this ticket

A central registry (D6: none).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
