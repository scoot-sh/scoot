---
title: "A user-facing docs site, with internal notes moved out of docs/"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# A user-facing docs site, with internal notes moved out of docs/

Filed 2026-10-03. Serves **daily-drive** (and adoption): the maintainer,
2026-10-03: "eventually redo the whole approach to docs in the repo and a
static docs site. Aimed at users, not technical updates."

## The gap

`docs/` serves two audiences at once. User reference (`configuration.md`,
`ipc.md`, `nix.md`, `tty.md`, `scootbar/cli.md`, `scootbg/cli.md`) sits beside
agent and maintainer material: the backlogs, benches, spikes, `forks.md`, the
resource ratchet's history, and reference pages that carry dated
measurement prose written for reviewers ("measured 2026-09-21 on the dev
VM..."). Nothing is published; a user reads raw Markdown on GitHub.

## What to do

- **Split by audience.** `docs/` becomes the site's source, written for
  users: what scoot is, install (Nix, FlakeHub once
  [nix-publishing](../resolved/nix-publishing-done.md) lands), a first session, configure,
  theming (Stylix), the bar, keybindings, the agent/IPC interface,
  troubleshooting. Backlogs, benches, spikes, forks and ratchet history move
  to a contributor tree that is not published (e.g. `dev/`), with every link
  in the repo, `CLAUDE.md`, `scripts/backlog` and the agent files updated.
- **Task-first pages with real screenshots** (a designed desktop: light
  theme, an icon on every module, a wallpaper; see `module-icons`), reference
  tables kept but led by what a user wants to do.
- **History out of user pages**: dated measurements and review narratives go
  to the contributor tree or commit messages; `CLAUDE.md`'s "docs in the same
  PR" rule is restated to target the user pages.
- **Generator: Astro Starlight** (maintainer's decision, 2026-10-04). Node
  enters the repo for the site only: pin it through the flake (a
  `buildNpmPackage`-style derivation with a locked `package-lock.json`, no
  global installs), keep it out of every Rust job's inputs, and give the site
  its own CI path filter. Theme it to look like scoot (the example looks'
  palettes, real screenshots), not stock Starlight.
- **LLM support from the first page** (maintainer, 2026-10-04): publish
  `/llms.txt` (the llmstxt.org index), `/llms-full.txt` (every page) and a
  small variant, plus each page as plain Markdown at a stable URL, generated
  by the build (e.g. the `starlight-llms-txt` plugin; verify it and pin it),
  so an agent can read the docs without scraping HTML. Pages are written so
  they read well as plain text too: commands in fenced blocks, no meaning
  carried only by images or tabs. CI checks the files exist and list every
  page.
- **Hosting**: GitHub Pages from `main` by its own workflow, which must
  trigger on `docs/**` explicitly (docs-only pushes start no workflow today,
  by design). Domain: the maintainer's call.
- Keep `README.md` short and user-facing, pointing at the site.

## Decisions for the maintainer

Decided 2026-10-04: Astro Starlight, with llms.txt support from the start.
Still open: the domain, and the page outline (a draft outline for review
is the first deliverable, before any move).

## Not in this ticket

API docs for the internal crates (rustdoc stays local), translations.
