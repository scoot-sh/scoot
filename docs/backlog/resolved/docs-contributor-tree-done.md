---
title: "Contributor tree: internal notes out of docs/ into dev/, links updated"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Contributor tree: internal notes out of docs/ into dev/, links updated

Filed 2026-10-07, split out of `docs-site` (which stays open until the site
work below is referenced): the site is live at https://www.scoot.sh/ with
llms.txt and CI gates, and the user-facing reference has moved to `site/`,
but the contributor half of that ticket's "What to do" is still in `docs/`.
Serves **computer use** secondarily (agents read `llms.txt`, not the repo),
and maintainers primarily: `docs/` still serves two audiences at once.

## The gap

`site/OUTLINE.md` (status 2026-10-05): "the `dev/` plan for the contributor
tree, whose move is still open." Still in `docs/` with no `dev/` tree
(checked 2026-10-07, `ls dev` → no such directory): benches and spikes,
`dev/forks.md`, the resource-ratchet history in
`docs/scootbar/backlog/lightest.md`, and dated measurement prose in user
pages. PR B (`docs/site-move`, #447) deliberately kept backlogs,
`docs/roadmap/` and `ROADMAP.md` in `docs/` — that stand-down is recorded in
`site/OUTLINE.md` and is not this ticket's to relitigate. The eight
`docs/*.md` stubs (`configuration.md`, `ipc.md`, `nix.md`, `tty.md`,
`protocols.md`, `scootbar/cli.md`, `scootbar/icons.md`, `scootbg/cli.md`)
still say they stay "only until the maintainer points
`.claude/agents/scoot-implementer.md` and `.claude/agents/scoot-reviewer.md`
at the new layout" — and those agent files still reference the old `docs/`
paths (checked 2026-10-07). `.claude/` is a protected path no agent may
write, so that half needs the maintainer.

## What to do

- Move the contributor material to the unpublished `dev/` tree per
  `site/OUTLINE.md`'s plan (benches, spikes, `forks.md`, ratchet history,
  dated measurement prose out of user pages), keeping backlogs/roadmap where
  PR B left them unless the maintainer says otherwise.
- Update every link the move breaks: in the repo, `CLAUDE.md`,
  `scripts/backlog`, and the `.claude/` agent files (maintainer's hand:
  protected path).
- Retire the eight `docs/*.md` stubs once the agent files point at `site/`.

## Not in this ticket

The `docs-site` ticket itself (site build, gates, hosting); the
`docs-site-asset-check` entry (its own open ticket); backlogs/roadmap
placement (decided in PR B).

## Resolution (2026-10-08, PR #545)

Conservative reading: backlogs, `docs/roadmap/` and `ROADMAP.md` stay
where PR B left them; only benches, spikes, `forks.md`, the contributor
guide and the research/testing notes moved to `dev/` (history-preserving
`git mv`; full list in the PR body). Every inbound reference updated
(relative Markdown links recomputed per file; prose, code comments, site
absolute URLs, flake/CI comments; `dev/**` added to the docs-only
push filters so dev-only pushes start no workflow; deploy workflow
deliberately not tracking `dev/`). `scripts/backlog` unchanged (roots
stay). The eight stubs stay until the maintainer repoints the protected
`.claude/` agent files (edits listed in the PR body). Ratchet history
(`scootbar/backlog/lightest.md`, open and ongoing) stays with the
backlog by the same stand-down; dated-measurement prose extraction out
of the staying user READMEs is follow-up, not this move.

Evidence: relative-`.md` link check over all 585 `*.md` files — 133
unique broken before, 133 after (delta only the two pre-existing broken
targets that moved with their files; zero broken involve `dev/`);
`scripts/backlog check` shows only its 3 pre-existing problems;
`nix build .#docs-site` exit 0 on the Asahi M2; CI green on the PR head.
