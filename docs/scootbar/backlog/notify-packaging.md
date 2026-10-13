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

deb (the `build-deb.sh` loop and `deb-meta.py` tables), rpm (`%check` with `--check`), Arch
(`scootnotify` and `-git`), each with the user unit, the activation file and an example
config; `install.sh --notify` (refusing while another daemon owns the name, naming it); the
site `/scootnotify/` section per `site/OUTLINE.md` with sidebar and `customSets`.

## Acceptance

Each package builds in CI's packaging jobs and installs on a clean container; the site builds
with the snippet gates green.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The default flip.
