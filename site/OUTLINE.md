# Docs site outline (proposal for review — phase 1, nothing moves yet)

This is the draft page outline the `docs-site` ticket asks the maintainer to
review **before** any existing doc is moved or rewritten. The scaffold beside
it (`site/`, Astro Starlight) proves the shape with 4 adapted pages; every
mapping below says which current `docs/*.md` sections feed each planned page.

Conventions used below: `docs/configuration.md#section` means "that section
moves (rewritten task-first) into the named page". "Stays" means the file
keeps living in the repo but is not published. "Moves to `dev/`" means it
leaves `docs/` for the unpublished contributor tree.

## User-facing page tree (`/`)

Top-level order is the order a new user reads in: install → first session →
daily use → customize → automate → fix → look up.

| Page (slug) | One-line purpose | Fed by (current sections) |
|---|---|---|
| `/` (home) | What scoot is, one screenshot, install hint, links into the tree | `README.md` Why/Get started (shortened); hero art `docs/assets/vinyl-sunset-preview.png` |
| `/install` | Get a working scoot on your machine, Nix first | `docs/nix.md` Consuming the flake, Prebuilt binaries/Cachix, FlakeHub, Platform notes; `README.md` Get started |
| `/first-session` | Boot to a desktop in 10 minutes: greeter login, first terminal, first keybindings | `docs/nix.md` What a greeter login starts, The greeter (opt-in); `docs/configuration.md` Starting a session, Default keybindings (first half); `docs/tty.md` intro (which backend am I on) |
| `/configure` | The config file: where it lives, reload, full section tour | `docs/configuration.md` The config file, Failure semantics, Reloading, `[layout]`, `[appearance]`, `[output]`/`[[outputs]]`, `[renderer]`, `[tty]`, `[xwayland]`, `[binds]`, `[autostart]`, `[floating]`, `[[window_rule]]`, Example config |
| `/keybindings` | Every default binding plus how to rebind | `docs/configuration.md` `[binds]`, Moving across outputs, Default keybindings; `nix/modules` live defaults referenced by `docs/nix.md` Reference: live defaults |
| `/theming` | Looks: pick one of the three examples, what each file does, what it costs | `docs/examples/{vinyl-sunset,music-desk,radial-burst}/README.md` + `scoot.toml`/`regreet.css` comments; `docs/nix.md` Stylix; `docs/configuration.md#appearance` cost notes |
| `/bar` | scootbar: enable it, pick modules, one annotated config | `docs/nix.md` The status bar; `docs/scootbar/cli.md` The config file, Layout, per-module sections (condensed to the daily-use subset); `docs/scootbar/icons.md` |
| `/wallpaper` | scootbg: solid color vs image, per-output, restore | `docs/nix.md` The wallpaper; `docs/scootbg/cli.md` Colors and images, Scale, One output or every output, Restore, What it costs |
| `/desktop` | The one-enable full desktop profile (`programs.scoot.desktop`, `look`) | `docs/backlog/packaging/desktop-paved-path.md` (the design, rewritten user-first) + each `desktop-*` child entry as its slot lands; `docs/nix.md` Home-manager/NixOS modules |
| `/agents` | Drive scoot from an agent: IPC socket, requests/actions, screenshots | `docs/ipc.md` (most of it: socket, Requests, Actions, Replies, Events, Rules an agent needs, Resource bounds); later generated CLI pages from `--help --json` (`agent-friendly-help.md` — slot reserved, see below) |
| `/troubleshooting` | "Black screen / no output / key not working" fixes in symptom order | `docs/tty.md` When nothing works, Hotplug, VT switching; `docs/configuration.md` Failure semantics; `docs/nix.md` Settings failure modes; `docs/scootbar/cli.md` `--check` |
| `/reference/` (section) | Complete, uncondensed reference tables the task pages summarize | Full `docs/configuration.md`, `docs/scootbar/cli.md`, `docs/scootbg/cli.md`, `docs/protocols.md` What is implemented (table), `docs/tty.md` Which renderer draws |
| `/reference/cli` (reserved slot) | **Not built in phase 1.** Generated from `--help --json` per `agent-friendly-help.md` | Future: `crates/{scoot,scootctl,scootbar,scootbg}/src/cli.rs` |
| `/reference/protocols` | Which Wayland protocols work and what each means for your apps | `docs/protocols.md` per-protocol sections (user half; measurement prose moves to `dev/`) |

Deliberately **not** pages: `docs/benchmarks.md` (dated measurements →
`dev/benches`), `docs/forks.md` (→ `dev/`, linked from the site footer
colophon at most), `docs/development.md` (→ `dev/` contributor guide),
`ROADMAP.md` (stays, linked from `dev/`).

## Contributor tree (`dev/`, unpublished)

`dev/` is built from the same repo but **never deployed**: no link from the
site nav, `exclude`d from every `llms*.txt` bundle, `robots: noindex` if it
ever renders. Proposed root:

```text
dev/
  README.md            # "you are a contributor; start here" (from docs/development.md)
  backlog/             # docs/backlog/** verbatim (open + resolved archive)
  scootbar-backlog/    # docs/scootbar/backlog/** (kept separate: separate README index)
  scootbg-backlog/     # docs/scootbg/backlog/**
  roadmap/             # docs/roadmap/** verbatim
  benches/             # docs/benchmarks.md + docs/scootbar/bench + docs/scootbg/bench
  spikes/              # docs/scootbar/spikes/**
  forks.md             # docs/forks.md verbatim
  ratchet/             # docs/scootbar/backlog/lightest.md history + resource prose
  research/            # docs/scootbar/research.md + docs/scootbar/testing.md + docs/scootbg/testing.md
```

What moves, exactly:

- `docs/backlog/**` (all areas + `resolved/` + `claims/`) → `dev/backlog/`
- `docs/scootbar/backlog/**`, `docs/scootbg/backlog/**` → `dev/scootbar-backlog/`, `dev/scootbg-backlog/`
- `docs/roadmap/**` → `dev/roadmap/`
- `docs/benchmarks.md`, `docs/scootbar/bench/**`, `docs/scootbg/bench/**` → `dev/benches/`
- `docs/scootbar/spikes/**` → `dev/spikes/`
- `docs/forks.md` → `dev/forks.md`
- `docs/scootbar/backlog/lightest.md`, `docs/scootbar/research.md`, `docs/scootbar/testing.md`, `docs/scootbg/testing.md`, `docs/development.md` → `dev/ratchet/`, `dev/research/`, `dev/README.md`
- Stays in `docs/` as site sources (rewritten in place, then published): `configuration.md`, `ipc.md`, `nix.md`, `tty.md`, `protocols.md`, `scootbar/cli.md`, `scootbar/icons.md`, `scootbg/cli.md`, `scootbar/README.md`, `scootbg/README.md`, `examples/**`, `assets/**`

## Every place that hard-codes those paths

The move PR must update all of these (this list is the audit; agents cannot
edit `.claude/*`, so those two are flagged for the maintainer):

1. `CLAUDE.md` — "Backlog" section (`docs/backlog/`, `docs/scootbg/backlog/`, `docs/scootbar/backlog/`), "Commits and CI" (`docs/backlog/packaging/independent-versioning.md`, `paths-ignore` note naming `docs/`), Smithay-fork note (`docs/backlog/resolved/syncobj-handle-leak-done.md`, `docs/backlog/core/smithay-fork-repin.md`, `docs/forks.md` ×2), protocol inventory (`docs/backlog/protocols/`), roadmap worked example (`docs/roadmap/05b-vt-switch-eperm.md`), per-feature-cycle docs rule (restate to target the user pages).
2. `.claude/agents/scoot-implementer.md` — "Update the docs" paragraph (names `docs/` files: `configuration.md`, `ipc.md`, `protocols.md`, `tty.md`, `scootbg/cli.md`). **Maintainer edit (protected path).**
3. `.claude/agents/scoot-reviewer.md` — "User-facing documentation" paragraph (names the same `docs/` references). **Maintainer edit (protected path).**
4. `.claude/skills/backlog/SKILL.md` — every `scripts/backlog` example path (`docs/backlog`, `docs/scootbg/backlog`, `docs/scootbar/backlog`, README links). Either the skill moves with the tool or the tool learns `dev/` roots. **Maintainer edit (protected path).**
5. `scripts/backlog` — `BACKLOGS` map (lines 38–46), `RESOLVED` (47–52), `INDEXES` (53–58), `CLAIMS_DIR = "docs/backlog/claims"` (line 177), the `ls-tree` paths in claim (278–279). Mechanical: repoint roots at `dev/`.
6. `.github/workflows/ci.yml` — `paths-ignore` (`**.md`, `docs/**`), classify `*.md | docs/*)` case + its long comment, backlog comment (`docs/backlog/claims/`, `docs/scootbar/testing.md` ×2). The site job added in phase 1 (`site/*` → `docs-site`) gains `docs/*` once docs feed the site.
7. `docs/backlog/README.md`, `docs/scootbar/backlog/README.md`, `docs/scootbg/backlog/README.md` — index links per entry (move with the entries; re-verify with `scripts/backlog check` + the `git diff origin/main...HEAD` line-count check from the brief).
8. In-prose links: `docs/nix.md` ↔ `docs/scootbar/cli.md#fonts`, `docs/configuration.md` ↔ `docs/scootbg/README.md`, example READMEs (`../../nix.md`, `../../configuration.md`, `../../tty.md`, `../../scootbg/README.md`, `../../scootbar/backlog/lightest.md`), `NOTICE` (names `docs/assets/` previews), `README.md` Documentation section, `ROADMAP.md` (links backlog entries throughout).
9. `flake.nix` comments naming `docs/` (nix.md, `docs/backlog/resolved/nix-crane-done.md`, `docs/scootbg/backlog/resolved/dependencies-done.md`) — comments only, but fix them in the move PR so they don't rot.
10. `vm/configuration.nix:150` (`docs/backlog/packaging/nix-publishing.md`) and `nix/tests.nix:53` (`docs/scootbar/backlog/resolved/nix-package-done.md`) — comment-only references, fixed in the move PR. (`nix/scootbar-tests.nix` has none; verified 2026-10-05.)

## Phase-1 scaffold (what `site/` holds today)

- `astro.config.mjs` — Starlight + `starlight-llms-txt` (pinned; see `site/README.md` "Why starlight-llms-txt"), sidebar matching the user tree above.
- `src/content/docs/` — 4 adapted (not moved) pages proving the shape: `index.md` (home), `install.md`, `first-session.md`, `keybindings.md`. Originals untouched in `docs/`.
- `src/styles/custom.css` — scoot theme from the vinyl-sunset palette (espresso `#271A1F`, cream `#F1E3C6`, sunset orange `#E59560`); system font stack (see below).
- `src/pages/[...page].md.ts` — per-page Markdown twins at stable `/<slug>.md` URLs (the plugin covers the aggregates; this covers the per-page requirement).
- `scripts/check-llms.mjs` — build gate: fails if `llms.txt` misses a page, a `.md` twin, or a linked bundle 404s.
- Fonts: **no font file is bundled.** The bar's look needs Droid Sans Mono Nerd Font Propo / FiraCode Nerd Font (glyphs + OFL licensing per face — vendoring binaries into the site is bloat and a license-attribution surface), so the site uses the system stack. Noted here so "the bar font if licensing allows" is answered: it allows (OFL), it just isn't worth it.
