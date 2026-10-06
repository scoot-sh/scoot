---
title: "Docs site HTML links to raw .md twins instead of pages"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution

Landed in PR #476 (`fix(site): link pages, not raw .md twins, in built
HTML`). One deviation from the plan, with evidence: the rewrite is a
post-build `astro:build:done` integration (`site/scripts/md-links.mjs`),
not a remark/rehype plugin. Two findings forced it: (1) Astro 7.3.5's
deprecated `markdown.remarkPlugins` never runs with Starlight in play
(proven with a throwing probe: clean build, no error, links unchanged);
the supported `processor: unified(…)` form does run. (2) That form
contaminates the llms.txt bundles — `starlight-llms-txt` renders pages
through the same markdown pipeline (`render(entry)` → HTML → Markdown,
verified in the pinned 0.12.0 source), so bundle prose picked up rewritten
`./scoot/keybindings/` links where agents must keep `.md` ones. Only
`dist/**/*.html` is touched now; twins and bundles verified `.md`-clean
in the other direction. Sidebar/`pagination` (root-absolute from slugs)
untouched; 404 handled (root `404.html`, links `./` and
`./scoot/keybindings/`).

Evidence: gate `check-md-links` fails on main with 287 failures, passes
after (293 links resolve); `nix build .#docs-site` green with no
`pnpm-lock.yaml` change; three Start-here links click through to HTML
200s; a project-pages `/scoot/` base build passes the same gate.
