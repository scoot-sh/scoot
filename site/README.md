# scoot docs site (phase-1 scaffold)

Astro Starlight + pnpm, built reproducibly through the flake
(`nix build .#docs-site`). The outline awaiting maintainer review is
[OUTLINE.md](OUTLINE.md); nothing in `docs/` has moved.

## Why Starlight, why this plugin

Generator: **Astro Starlight** (maintainer decision, 2026-10-04). Why
`starlight-llms-txt` (delucis, MIT, pinned `0.12.0`, ~150k weekly
downloads, listed in Starlight's own plugin directory) for the
machine-readable surface — evaluated 2026-10-05 against:

| Candidate | Verdict |
|---|---|
| `starlight-llms-txt` | **Chosen.** Generates exactly the ticket's three files — `/llms.txt` (index), `/llms-full.txt` (everything), `/llms-small.txt` (small variant) — prerendered at build time, with promote/demote/exclude plus per-app `customSets` (one set per sidebar section with pages, served at `/_llms-txt/<set>.txt`). Small, Starlight-aware (sidebar order), maintained. |
| `@wave-rf/starlight-llm-tools` | Rejected. Does per-page `.md` + indexes, but tiny adoption, and drags in UI chrome (copy-markdown buttons, AI dropdowns) nobody asked for. |
| `starlight-llm-actions` | Rejected. Same reason: page-action dropdowns and renderers we don't need. |
| `astro-slop`, `astro-markdown-for-agents` | Rejected. Generic Astro integrations, not Starlight-aware; more hand-rolling for less. |

The plugin does **not** do per-page Markdown, so the scaffold adds its own
tiny route instead of a second dependency: `src/pages/[page].md.ts` serves
each doc's raw body at a stable `/<slug>.md` URL (`/index.md` for home).
~30 lines, no new dependency, covered by the `check-llms` gate below.

## Add a page

1. Write `src/content/docs/<slug>.md` — plain Markdown only, no MDX
   components (the `.md` twin serves the raw body; MDX imports would leak
   into it). Small raw-HTML accents are allowed where Markdown cannot do
   the job (the hero art, the home Looks grid, the 404 terminal chrome),
   but they must keep the twin readable: real links, real alt text.
   Need tabs, steps, asides or cards? Write `<slug>.mdx` instead (`Tabs`/`TabItem`, `Steps`, `Aside`, `LinkCard`/`CardGrid` from `@astrojs/starlight/components`): the twin route converts it to plain Markdown (imports stripped, `TabItem` → `###`, `Aside` → `> **Note:**`, `LinkCard` → link list; fences untouched) and `pnpm test-mdx-twin` plus the llms gate fail on any leftover `<[A-Z]` JSX — so backtick-quote generics like `` `<N>` ``.
2. Add it to its app's `sidebar` group in `astro.config.mjs` (new app, new
   group per `OUTLINE.md`), to `promote` if it should sort near the top
   of the bundles, and to that app's `customSets` entry (new app, new
   set).
3. Page contract (the docs-bar standard, phase-1 slice): task-first lead,
   option tables with type/default/live-reload/example, copy-paste-correct
   snippets (`toml` must parse, `sh` must pass `bash -n` — enforced by
   `pnpm test-snippets`), symptom boxes (`> **Symptom:** …`), links both
   ways between concepts and options.
4. `pnpm verify` before pushing: build + llms gate + snippet gate.

## Gates

- `pnpm check` (`scripts/check-llms.mjs`): fails the build if `llms.txt`
  misses a page, a `.md` twin is missing/empty, or a linked bundle 404s.
  Runs in the nix derivation after `astro build` and in CI. Covers the
  whole tree (`src/content/docs/**/*.{md,mdx}`, nested sections included);
  `.mdx` twins additionally fail on any `<[A-Z]` JSX left in the prose.
- `pnpm check-md-links` (`scripts/check-md-links.mjs`): fails the build
  if any built HTML page links a raw `.md` twin (a relative `href` ending
  in `.md` or `.md#…`), or if a rewritten page link resolves to no page
  in `dist/`. The rewrite itself is the `mdLinksToPages` integration
  (`scripts/md-links.mjs`, `astro:build:done`): `foo/bar.md` → `foo/bar/`,
  `foo/index.md` → `foo/`, anchors and query strings kept, correct under
  any `base`. It touches `*.html` only — the `.md` twins and the llms.txt
  bundles keep their `.md` links for agents (a remark/rehype plugin cannot:
  the bundles render through the same markdown pipeline, so the rewrite
  would leak into them; verified 2026-10-06).
- `pnpm check-assets` (`scripts/check-assets.mjs`): fails the build if any
  CSS `url()` (`dist/**/*.css`, plus `<style>` blocks and `style=""`
  attributes in `dist/**/*.html`) or any HTML `src`/`href`/`srcset` naming
  an asset (images, fonts, CSS/JS, icons) resolves to no file in `dist/`.
  Resolution is base-aware (root-absolute URLs resolve under the built
  base, inferred from the `/_astro/` prefix; page-relative URLs against
  the linking file, CSS-relative against the stylesheet). Intentional
  `.md` twin links are excluded (the llms gate covers those), as are
  `.txt` bundles, feeds, page links and external/data URLs. Vite leaves a
  CSS `url()` untouched when its file is absent and still exits 0 (PR
  #466 shipped an `@font-face` that would 404 this way), so without this
  gate a missing font or image is a green build. `pnpm test-check-assets`
  pins the fixture (`scripts/fixtures/check-assets/`): one missing font
  plus one missing image must fail, the all-present twin must pass.
- `pnpm test-snippets`: toml/sh soundness + the "color"/"compositor"
  wording rules, over the whole tree (`*.md` and `*.mdx`). Executing
  snippets against headless scoot is the recorded next step (needs the
  Linux binary; both runners already call this file).
- `pnpm test-mdx-twin` (`scripts/test-mdx-twin.mjs` over the fixtures in
  `scripts/fixtures/mdx-twin/`, never published): the good `.mdx` fixture
  (Tabs with `syncKey`, Steps, Aside, LinkCard, TOML fence) must convert
  to plain Markdown with fences intact, and the broken fixture's twin must
  trip the `<[A-Z]` JSX gate — both directions, every run.
- `pnpm test-nix` (`scripts/check-nix.mjs`): every fenced `nix` block
  must parse under `nix-instantiate --parse` — as a full file, a closed
  fragment, or a module body with `inputs`/`pkgs`/`lib`/`config` bound —
  so the flake/module snippets cannot rot at the syntax level. Runs in
  the derivation (which provides `nix`) and anywhere with `nix` on
  `PATH`. Deeper — that the documented NixOS and home-manager desktop
  configs actually evaluate — is pinned in `nix/tests.nix` ("the
  documented install configs", run by `nix flake check` in CI); keep
  that test in step with `desktop/index.md` when the page changes.

## Nix build

`nix/docs-site.nix`: `fetchPnpmDeps` (hash-pinned `pnpm-lock.yaml`) +
`pnpmConfigHook` + `astro build` + the llms gate, offline. No network at
build time beyond the deps hash. `site` and `base` both derive from the
single `DEPLOY_TARGET` value in `astro.config.mjs` (`'www'` today:
`https://www.scoot.sh/` with base `/`; see "The domain").

## The domain

Live at **https://www.scoot.sh/** (2026-10-05): Cloudflare DNS has
`www.scoot.sh` as a DNS-only CNAME to `scoot-sh.github.io`, the Pages custom
domain is `www.scoot.sh` (`gh api -X PUT repos/scoot-sh/scoot/pages -f
cname=www.scoot.sh`), and `DEPLOY_TARGET = 'www'` in `astro.config.mjs` sets
`site: https://www.scoot.sh/` and `base: /` together. The old
`scoot-sh.github.io/scoot/` address redirects there.

The bare `scoot.sh` is not served by Pages yet (it is proxied by Cloudflare).
To serve or redirect it: either a Cloudflare redirect rule `scoot.sh/*` →
`https://www.scoot.sh/$1` (keeps `www` canonical, nothing changes here), or
point the apex at GitHub Pages too (GitHub then redirects it to the custom
domain). Moving to Cloudflare Pages later changes only the deploy job and
the DNS target, not `DEPLOY_TARGET`.
