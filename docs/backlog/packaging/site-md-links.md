---
title: "Docs site HTML links to raw .md twins instead of pages"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Docs site HTML links to raw .md twins instead of pages

Filed 2026-10-06. Serves **daily-drive**: a user reading the published
docs clicks a link and lands on raw Markdown instead of the next page.

## The gap

The sources write relative `.md` links (good for editors, GitHub and the
raw twins), e.g. `site/src/content/docs/index.md:50`
`[Install](./start/install.md)`. The site also serves raw Markdown twins
at `.md` URLs (`site/src/pages/[...page].md.ts`), so the built HTML keeps
the `.md` href verbatim and a human clicking it lands on raw Markdown.
On the live www.scoot.sh the home page renders
`href="./start/install.md"`, `./start/first-session.md`,
`./scoot/keybindings.md`, `./desktop/index.md`,
`./start/install.md#which-build-do-i-need`; `scoot/keybindings/` renders
`../desktop/index.md#launcher`. Site-wide: 302 `.md` link occurrences in
`site/src/content/docs/`.

## What to do

- Rewrite relative `.md` links in the HTML render only (remark/rehype
  plugin in `site/astro.config.mjs`): `foo/bar.md` → `foo/bar/`,
  `foo/index.md` → `foo/`, keep `#anchors` and query strings, stay
  correct under the project-pages `base`, leave absolute/external/
  non-`.md` links alone.
- Raw `.md` twins and llms.txt keep `.md` links (agents reading Markdown
  follow Markdown); the plugin must not touch them.
- Gate in the docs-site build (beside the llms/snippet gates): fail if
  any built HTML page has a relative `href` ending in `.md`/`.md#…`;
  every rewritten link resolves to a page in `dist/`.
- Check Starlight's own sidebar/`pagination` links and the 404 page are
  not affected.

## Not in this ticket

Changing the sources to extensionless links (they must stay `.md` for
editors, GitHub and the twins); the `##` double-hash anchors found in
several pages (valid fragments, out of scope).
