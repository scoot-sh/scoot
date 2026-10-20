---
title: "scootnotify: deb, rpm and Arch packages, install.sh, site section"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-20"
---

# scootnotify: packaging and the site

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#docs-and-packaging). Serves **daily-drive**.

## What to do

- **Packages**, each with the user unit, the activation file and an example config:
  - deb: the `build-deb.sh` loop and the `deb-meta.py` tables;
  - rpm: `%check` with `--check`;
  - Arch: `scootnotify` and `-git`.
- **`install.sh --notify`**, refusing while another daemon owns the name, and naming it.
- **The site's `/scootnotify/` section**, per `site/OUTLINE.md`, with its sidebar entry and
  `customSets`.

## Acceptance

Each package builds in CI's packaging jobs and installs on a clean container, and the site
builds with the snippet gates green.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/start/ (install pages) and site/src/content/docs/scootnotify/index.md` with per-distro install of scootnotify. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip.

## Resolution

Shipped in PR #699 (`feat(scootnotify,packaging): deb, rpm and Arch
packages, install.sh --notify, site section`):

- deb: `scootnotify` in `deb-meta.py` (`ALLOW` gains nothing past
  libc: the daemon links nothing past the C library;
  `Suggests: fonts-dejavu-core`), built by `build-deb.sh` with the
  binary, the user unit, the D-Bus activation file and
  `notify.toml.example`, and pinned by `test_deb.py` (21/21 green:
  the libc-only shlibdeps passes, the compositor and keyboard
  libraries fail).
- rpm: `packaging/rpm/scootnotify.spec` (binary, `scootnotify.service`
  with `%systemd_user_post`/`%systemd_user_preun`, the activation
  file, `notify.toml.example` as `%doc`; no system-library
  `BuildRequires`; `%check` runs `--version` and `daemon --check`,
  plain and on the shipped example).
- Arch: `packaging/arch/scootnotify/` and `scootnotify-git/` release
  and `-git` PKGBUILDs (`depends` is `glibc` plus `libgcc` only, no
  bfd pin; `optdepends` on `systemd` and `ttf-dejavu`); an `.install`
  note names the single-owner rule for the bus name. Both `.SRCINFO`
  files committed to match `makepkg --printsrcinfo` (proven by the
  arch.yml gate).
- Container workflows (`deb.yml`, `rpm.yml`, `arch.yml`) build, lint,
  install and prove the daemon (`--version`, `daemon --check` on the
  shipped example under `dbus-run-session`, plus a `Notify` +
  `msg list --json` smoke against the running daemon on a session
  bus); `release.yml` publishes the portable `scootnotify` binary
  (five binaries, thirteen assets) and verifies it.
- `scripts/sync-packaging-versions` covers the new recipes (plus a
  missing-file refusal, pinned by two new tests);
  `scripts/release-resolve` gains the fifth binary (asset count 11
  -> 13 in tests).
- `install.sh --notify` (opt-in until the default flip): record,
  up-to-date, uninstall (disabling the unit first), enable,
  font-report and `--system` plumbing mirroring `--launch`, plus the
  owner gate: refuses while a foreign process owns
  `org.freedesktop.Notifications`, naming `comm (pid N)` via busctl
  (dbus-send fallback), and never stops another daemon. Covered by
  `test-install.sh` (all passed, six new tests).
- Site: per-distro install on the scootnotify overview, `--notify` on
  the install page, the compose row, and the releases, packaging and
  cut-a-release rows. Sidebar, `customSets` and OUTLINE already
  covered `/scootnotify/`.
