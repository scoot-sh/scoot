---
title: "A user-facing docs site, with internal notes moved out of docs/"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
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
Decided 2026-10-04: pnpm, not npm, for anything Node/TypeScript
(`pnpm-lock.yaml`, flake-built with `fetchPnpmDeps` + `pnpmConfigHook`,
`packageManager` pinned in `package.json`).
Still open: the domain, and the page outline (a draft outline for review
is the first deliverable, before any move).

## Phase 1 progress (scaffold, 2026-10-05 — ticket stays open)

Branch `docs/site-scaffold` (PR forthcoming): `site/OUTLINE.md` (user +
contributor trees, the path-hard-coding audit), a building Starlight
scaffold (`nix build .#docs-site`: 4 adapted pages, vinyl-sunset theme,
`starlight-llms-txt` 0.12.0 pinned for the three bundles plus a first-party
`/[page].md` route for per-page twins, `check-llms` + `test-snippets`
gates), a path-filtered `docs-site` CI job, and a drafted-but-disabled
Pages deploy workflow. Next: maintainer reviews the outline; then the move
(`dev/` tree + link updates), executing `test-snippets` against headless
scoot (parse/syntax only so far), generated CLI pages from `--help --json`,
and reproducible IPC screenshots per the docs-bar standard.

## Phase 2 PR A: deploy (2026-10-05 — ticket stays open)

Maintainer 2026-10-05: approved the outline with the scoot desktop as the
primary path and three front doors (the scoot desktop / just the
compositor / agents-headless-webtop); hosting GitHub Pages now,
Cloudflare later (`scoot.sh` owned on Cloudflare). Branch
`ci/docs-site-deploy`: the deploy workflow is on (push to `main` on
`site/**`, `docs/**`, `nix/docs-site.nix`; `nix build .#docs-site`,
upload artifact, `actions/deploy-pages` SHA-pinned, `environment:
github-pages`, least-privilege permissions), base stays `/scoot/` so it
serves at `scoot-sh.github.io/scoot/`; `site`+`base` derive from one
`DEPLOY_TARGET` value and `site/README.md` records the `scoot.sh` switch
steps. Proven with a `workflow_dispatch` dry run on the branch: build
green, `github-pages` artifact uploaded; the deploy step itself waits for
`main` (the `github-pages` environment's branch policy). Next: PR B (the
move) after A merges; resolve this ticket only when the site is live on
`scoot.sh` (not yet).

## Phase 2 PR B: the move (2026-10-05 — ticket stays open)

Branch `docs/site-move`: user-facing material moved into the site's
per-app sections (Start / the scoot desktop / scoot / scootctl & IPC /
scootbar / scootbg / agents & webtop / troubleshooting / reference),
task-first, option tables with type/default/example/reload, symptom
troubleshooting, the desktop as the primary path with the three front
doors, the "Which build do I need?" GPU chooser (one-command check,
verified against code), copy-paste-complete flake/NixOS/Home Manager
setup, keybinding cheat sheet, per-app llms.txt sets for every section.
Snippet gates extended to the whole tree (`test-snippets` recurses,
new `check-nix` parses every nix block in the derivation, and
`nix/tests.nix` evaluates the documented minimal NixOS + standalone
HM desktop configs). Backlogs, `docs/roadmap/` and `ROADMAP.md` stay
(the `.claude/` edits for their move are listed in the PR). Resolve
this ticket only when the site is live on `scoot.sh` (not yet).

## Not in this ticket

API docs for the internal crates (rustdoc stays local), translations.

## Status (2026-10-07, hygiene pass — ticket stays open)

Live at https://www.scoot.sh/ (homepage + `/llms.txt` with small/full and
per-section sets fetched 2026-10-07; `DEPLOY_TARGET = 'www'` in
`site/astro.config.mjs`). User-facing reference moved to `site/` (the eight
`docs/*.md` are stubs pointing there); gates green in `nix/docs-site.nix`
(`check-llms`, `check-md-links`, `test-snippets`, `check-nix`) with the
path-filtered CI job and the Pages deploy workflow. What keeps this ticket
open: the contributor-tree half (`dev/` move + repo-wide link updates,
`.claude/` agent files need the maintainer — split to
`docs-contributor-tree.md`) and the asset check (its own open entry
`docs-site-asset-check.md`). The "(not yet)" notes above are stale and kept
for history.

## Resolution (verify-first pass, 2026-10-08 — ticket done, resolving)

Re-checked every "What to do" bullet against the site as built and as
deployed; everything this ticket asked for is landed. The two remainders
named in the status above both live elsewhere now: the contributor-tree
half is `docs-contributor-tree.md` (a separate open ticket, not this
one), and the asset check resolved 2026-10-07 in #504
(`resolved/docs-site-asset-check-done.md`). No new work in this PR —
docs-only resolve.

- Split by audience (user half): `site/src/content/docs/` holds 49
  pages across Start, desktop, scoot (13 pages), msg (6), scootbar (6),
  scootbg (9), agents, troubleshooting, reference (6); the eight
  `docs/*.md` are stubs pointing there. Contributor half: untouched
  here, tracked by `docs-contributor-tree.md`.
- Task-first pages with screenshots: three front doors on the home
  page (desktop / compositor / agents-headless-webtop), Looks strip,
  GPU chooser and cheat-sheet pages live; `msg/screenshots.md`
  documents the reproducible IPC screenshot path.
- History out of user pages: `CLAUDE.md`'s docs rule targets the site
  pages. Dated measurement prose still inside user pages is
  `docs-contributor-tree.md`'s to move, not this ticket's.
- Generator: Astro Starlight (Starlight v0.42.5 on the live home
  page), pnpm via `fetchPnpmDeps` + `pnpmConfigHook` in
  `nix/docs-site.nix` (Rust jobs untouched), own CI path filter
  (`site/*` → `docs-site` in `ci.yml`; `*.md | docs/*` starts
  nothing), dark-only scoot theme (`custom.css`, no stock Starlight).
- LLM support: live `/llms.txt` (index + small/full + 9 per-section
  custom sets, fetched 2026-10-08), per-page `*.md` twins via
  `src/pages/[page].md.ts`, gates `check-llms` (49 pages with twins),
  `check-md-links` (347 page links), `check-assets` (550 refs),
  `test-snippets`, `check-nix` (49 blocks) — all green in
  `nix build .#docs-site` on the M2 2026-10-08 (exit 0).
- Hosting: `docs-site-deploy.yml` pushes to Pages on `site/**`,
  `docs/**`, `nix/docs-site.nix` (the explicit `docs/**` trigger the
  ticket required); `DEPLOY_TARGET = 'www'`, live at
  https://www.scoot.sh/ with Starlight search (Pagefind index built;
  `site-search` + `data-pagefind-body` on the live home page).
- README points at the site throughout (install, desktop, IPC,
  keybindings, protocols links to `www.scoot.sh`).

Evidence: `nix build -L .#docs-site` exit 0; `nix flake show
--all-systems --json --no-write-lock-file` exit 0; `scripts/backlog
check` shows only the 3 known pre-existing problems
(`protocol-gaps-general.md`, `protocol-gaps-niche.md`,
`multi-output-foundation-done.md`); `cargo deny check` —
advisories/bans/licenses/sources ok.
