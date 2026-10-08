---
title: "The docs-site build stays green when a referenced asset is missing"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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

## Resolution (PR #504, 2026-10-07)

Landed as filed: `site/scripts/check-assets.mjs` runs in
`nix/docs-site.nix` beside the llms and snippet gates (wired through
`site/package.json`'s `verify`), resolving every CSS `url()` and every
HTML `src`/`href`/`srcset` in `dist/`, base-aware (root-absolute under
the built base inferred from the `/_astro/` prefix; relative against
the linking file), scoped to images/fonts/CSS-JS/icons with the `.md`
twins excluded. `site/scripts/test-check-assets.mjs` pins the fixture
(`site/scripts/fixtures/check-assets/`): one missing font plus one
missing image must fail (2 missing), the all-present twin must pass.
The `src` filter now excludes only the site-root `dist/` so the
fixtures reach the sandbox.

Evidence (Asahi M2): doctored dist (font + hero image deleted) —
`check-llms` ok (48 pages), `check-md-links` ok (330 links),
`check-assets` FAIL naming exactly the two; baseline dist —
`check-assets` ok (537 asset references, 0 missing). `nix build
.#docs-site` green with all six gates; `cargo deny check`
licenses+advisories green; `nix flake check` green.
