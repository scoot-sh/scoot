---
title: "scootnotify: deb, rpm and Arch packages, install.sh, site section"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-nix"
milestone: "M7"
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
