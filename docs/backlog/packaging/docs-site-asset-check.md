---
title: "The docs-site build stays green when a referenced asset is missing"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# The docs-site build stays green when a referenced asset is missing

Filed 2026-10-06 from the PR #466 review. Serves **daily-drive**: a
broken font or image on www.scoot.sh is the first thing a new user sees.

## The gap

Vite leaves a CSS `url()` untouched when its file is absent, and the
build exits 0. During PR #466 a new font file was referenced before it
was staged in git, so the nix build (which only sees tracked files)
shipped an `@font-face` that would 404, and every gate passed.

## What to do

Add an asset-reference check to the scripts `nix/docs-site.nix` runs
(`site/scripts/`), beside the llms and snippet gates:

- Resolve every CSS `url()` and every HTML `src`, `href` and `srcset`
  in `dist/`, base-aware.
- Scope it to assets: images, fonts, CSS/JS, icons. Exclude the
  intentional `.md` twin links. The #466 reviewer's naive version
  false-positived on 202 of them; the llms gate already covers those.
- The #466 reviewer measured 2318 references and 0 missing on that
  head. Pin a test fixture with one missing font and one missing image
  that must fail the check.

## Not in this ticket

External links (that's a link checker, with network).
