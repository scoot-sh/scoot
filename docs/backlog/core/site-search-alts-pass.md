---
title: "Site: verify search ranking in a browser; descriptive preview alts"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Site: verify search ranking in a browser; descriptive preview alts

Filed 2026-10-22 (W48 docs-site UX pass). Serves **computer use**:
agents reach the docs through search and through the llms.txt sets, so
the top queries must land on the right page.

## The gap

1. Search ranking unverified. Every one of the top 10 queries
   (`install`, `docker`, `config`, `keybindings`, `looks`,
   `wallpaper`, `notifications`, `launcher`, `ipc`, `nix`) matches
   content on the expected page (checked by grep over
   `site/src/content/docs` in W48: `install` → `start/install.mdx`,
   `docker` → install + `agents/webtop.md`, `wallpaper` →
   `scootbg/**`, etc.), and Pagefind builds clean, but ranking was
   never checked in a real browser — W48 had no browser tools, only
   headless Lighthouse. If any query misses, the fix is Pagefind
   `synonyms`/`keywords`, not new prose.
2. Weak alt text (left alone in W48: the look/theming pages belong to
   W47's in-flight edit). `site/src/content/docs/scoot/theming.md`
   uses bare look names as alt (`![ginger-night]`, `![music-desk]`,
   …) while every other preview on the site describes the picture
   (e.g. the Looks strip on `index.md`). Also confirm the hero image
   carries explicit `width`/`height` (it ships `fetchpriority="high"`
   but no dimensions in the markup W48 read — the front page is
   frozen post-#720, so verify first, then decide).

## Progress

- Part 2, done 2026-11-07: the five previews in `scoot/theming.md` now
  carry alts that describe each picture (checked against the images).
  The hero image's `<img>` in the `index.md` frontmatter still has no
  `width`/`height` (the file is 960x640). Left as is: the front page is
  frozen post-#720, and with `.hero-art img { width: auto; max-height:
  42vh }` at desktop widths the change could not be judged without a
  browser. Add `width="960" height="640"` once it can be checked at
  both widths.
- Part 1 (search ranking in a real browser): still open.

## What to do

Drive the built site in a real browser at desktop and phone widths,
type each of the 10 queries into search, and record which page ranks
first. Fix misses with the smallest lever (frontmatter keywords,
a heading that names the thing, Pagefind synonyms). Give the five
theming previews descriptive alts matching the Looks-strip style.
Re-run `pnpm verify` and Lighthouse after.

## Not in this ticket

New search infrastructure or a new JS dependency — Pagefind ships
with Starlight; lightweight is a product value.
