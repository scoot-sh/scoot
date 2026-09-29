---
name: backlog
description: List, read, file, edit and resolve scoot backlog entries (docs/backlog and docs/scootbg/backlog) with scripts/backlog. Use when asked what is left, what to pick next, what is blocked, to file or triage a ticket, or to mark one resolved.
---

# Working the backlog

Entries are markdown files with frontmatter (`title`, `status`, `area`,
`priority`, `blocked`). Use `scripts/backlog` rather than hand-editing
frontmatter or grepping. It is stdlib Python, and every edit touches only the
frontmatter line it names.

## Look

```sh
scripts/backlog list                         # everything open, high priority first
scripts/backlog list --unblocked --priority high
scripts/backlog list --blocked               # what waits on something
scripts/backlog list --area scootbg          # core | ipc | protocols | testing | scootbg
scripts/backlog list --grep 'ext-workspace'  # match anywhere in the file
scripts/backlog list --all                   # include the resolved archive
scripts/backlog show workspace-snapshot      # slug, unique substring or path; --full for all
```

Pick the next item by `CLAUDE.md`'s framing: computer use and daily-driving
both count, and the case for an item should say which it serves.

## File

```sh
scripts/backlog new ipc my-slug --title "One line" --priority medium [--blocked "waits on X"]
```

Then fill in the template (what is wrong with evidence, what to do, what is
out of scope), and **link the entry from `docs/backlog/README.md`**
(`docs/scootbg/backlog/README.md` for scootbg). Check the claims against the
code before writing them; entries here have been wrong before.

## Edit

```sh
scripts/backlog set SLUG priority=high blocked=null status=research
```

Fields: `title`, `status` (`open|research|resolved`), `area`, `priority`
(`high|medium|low|research|null`), `blocked` (text, or `null` for nothing).

## Resolve

```sh
scripts/backlog resolve SLUG
```

Sets `status: resolved`, clears priority and blocked, and moves the file to
`resolved/<slug>-done.md` (`git mv` when tracked). It does **not** write the
resolution: add what landed, the evidence and the PR to the entry's prose, then
fix every link the move broke (`grep -rn SLUG docs ROADMAP.md README.md`).
Resolved entries keep their diagnosis history; do not rewrite them.

## Check

```sh
scripts/backlog check
```

Validates frontmatter, that `resolved/` and `status` agree, that `area`
matches the directory, and that each open entry is linked from a README or
`ROADMAP.md`. Run it before committing backlog changes. Exit status is 1 on
any problem.
