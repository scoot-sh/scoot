---
title: "scootlaunch: deb, rpm and Arch packages, install.sh, site section"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootlaunch: packaging and the site

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#docs-and-packaging). Serves **daily-drive**.

## What to do

- deb, rpm and Arch packages.
- `install.sh --launch`.
- The site's `/scootlaunch/` section, with the dmenu flag table, troubleshooting boxes, its
  sidebar entry and `customSets`.

## Acceptance

The packages build and install in CI's containers, and the site builds with the snippet gates
green.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/start/ (install pages) and site/src/content/docs/scootlaunch/index.md` with per-distro install of scootlaunch and its dependencies. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip.

## Resolution

Shipped in PR #682 (`feat(scootlaunch): deb, rpm and Arch packages,
install.sh --launch, site section`):

- deb: `scootlaunch` in `deb-meta.py` (`ALLOW` gains only
  `libxkbcommon0` past libc; `Suggests: fonts-dejavu-core`), built by
  `build-deb.sh` with binary plus licenses, and pinned by `test_deb.py`
  (18/18 green: the keyboard-library shlibdeps passes, a compositor
  library fails).
- rpm: `packaging/rpm/scootlaunch.spec` (binary plus licenses,
  `BuildRequires: libxkbcommon-devel`, `%check` runs `--version` and
  `--check` on the built-in defaults).
- Arch: `packaging/arch/scootlaunch/` and `scootlaunch-git/` release
  and `-git` PKGBUILDs (`depends` gains only `libxkbcommon`, plus a
  `ttf-dejavu` optdepend); both `.SRCINFO` files verified
  byte-identical against real `makepkg --printsrcinfo` output.
- Container workflows (`deb.yml`, `rpm.yml`, `arch.yml`) build, lint,
  install and prove the launcher (`--version`, `--check`) beside the
  rest; `release.yml` publishes the portable `scootlaunch` binary with
  a `libxkbcommon0` bare-base gate.
- `install.sh --launch` (opt-in until the default flip): record,
  up-to-date, uninstall and font-report plumbing mirroring the bar
  flags, covered by `test-install.sh` (all passed).
- Site: the panel page joins the sidebar (it existed but was
  unlisted), per-distro install on the overview, `--launch` on the
  install page, the launcher row in compose, and the releases and
  packaging reference rows. `customSets` already covered
  `scootlaunch/**`; the dmenu flag table and `Symptom` boxes were
  already in place from earlier PRs.
