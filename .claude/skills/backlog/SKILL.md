---
name: backlog
description: List, read, file, edit, claim and resolve scoot backlog entries (docs/backlog, docs/scootbg/backlog and docs/scootbar/backlog) with scripts/backlog. Use when asked what is left, what to pick next, what is blocked, to file or triage a ticket, to claim a ticket before working it (agent swarms), or to mark one resolved.
---

# Working the backlog

Entries are markdown files with frontmatter (`title`, `status`, `area`,
`priority`, `blocked`, optional `milestone`, and `resolved` once done). Use
`scripts/backlog` rather than hand-editing frontmatter or grepping. It is
stdlib Python, and every edit touches only the frontmatter line it names.

## Look

```sh
scripts/backlog list                         # everything open, high priority first
scripts/backlog list --ready                 # open, unblocked and not claimed: what to pick up
scripts/backlog list --unblocked --priority high
scripts/backlog list --blocked               # what waits on something
scripts/backlog list --area scootbar --milestone M1
scripts/backlog list --area scootbg          # core | ipc | protocols | testing | packaging | scootbg | scootbar
scripts/backlog list --fetch --ready         # refresh from main first; without --fetch, claims are as of your last fetch
scripts/backlog list --claimed               # who holds what, and for how long
scripts/backlog list --grep 'ext-workspace'  # match anywhere in the file
scripts/backlog list --all                   # include the resolved archive
scripts/backlog show workspace-snapshot      # slug, unique substring or path; --full for all
```

Pick by `CLAUDE.md`'s framing: computer use and daily-driving both count, and
the case for an item should say which it serves.

## Claim before you work (agent swarms)

Several agents, in any clones or on any machines, can work the backlog at once
without colliding. **Claim a ticket before you start it**: the claim is pushed
to `main`, so everyone sees it, and it is checked against a fresh fetch of
`main` before anything is picked up.

```sh
scripts/backlog claim --next --agent NAME --json        # fetch main, pick the top ready ticket, push the claim
scripts/backlog claim --next --area scootbar --milestone M1 --agent NAME --json
scripts/backlog claim SLUG --agent NAME                 # or a specific one
scripts/backlog renew SLUG                              # long job: re-push it so it stays live
scripts/backlog release SLUG                            # giving up: push its removal
scripts/backlog claims [--prune]                        # every claim on main; --prune drops dead ones
```

- **What a claim is**: a small file, `docs/backlog/claims/<area>__backlog__<slug>.json`,
  added to `main` in its own commit (`backlog: claim <slug> (<agent>) [skip ci]`)
  holding a uuid, your name and the date. The commit is built with git
  plumbing on top of the fetched `main`: **it never touches your working tree
  or your checked-out branch.** The push is the lock, so if two agents race,
  exactly one push wins and the other re-checks and is refused (or moves to the
  next ticket with `--next`).
- **The check happens against `main`, not your checkout**: readiness (open,
  unblocked) and other agents' claims are read from a fresh fetch, so a stale
  checkout cannot double-claim, and a ticket unblocked by a merge you have not
  pulled yet is still seen as ready.
- **Only entries that are on `main`** can be claimed; push a new entry first.
- **`--next` picks** the top open, unblocked, unclaimed entry: lowest milestone
  first (`M0`, `M1`, ...; entries with none last), then priority. Research
  entries only with `--research`.
- **Exit codes**: `0` claimed; `2` refused (someone holds it; says who and for
  how long); `3` nothing ready; `4` could not fetch or push (no network, or
  `main` is protected against direct pushes: claims need push access to it). On
  `3`, stop or widen the filters. Never `--force` a live claim unless the user
  said to.
- **The uuid** is in the output (`--json`), and is also kept in a **gitignored
  local record** in this worktree's git directory (`.git/backlog-claims.json`,
  per worktree, never committed), which is how `renew`, `release` and `resolve`
  know a claim is yours without `--token`. Pass `--token UUID` from another
  worktree or clone.
- **A claim expires** after 24 hours (`--ttl HOURS`, or
  `BACKLOG_CLAIM_TTL_HOURS`), so a crashed agent does not hold a ticket forever;
  a stale claim can simply be claimed again. Claims on tickets that are already
  resolved, and stale ones, are swept out by the next claim commit
  (`claims --prune` does it on demand).
- **Overrides**: `BACKLOG_REMOTE` (default `origin`) and `BACKLOG_CLAIM_BRANCH`
  (default `main`).
- **A claim is advisory, not a lock on files.** Two tickets can still touch the
  same files. Follow `CLAUDE.md`: each concurrent agent works in its **own git
  worktree** with its **own `CARGO_TARGET_DIR`**, and does not run
  branch-mutating git commands in a shared checkout.

A swarm loop, per agent: `claim --next` → read the entry (`show SLUG --full`) →
worktree and branch → implement through the full cycle → review → PR → after it
merges, `resolve SLUG` (in a follow-up or the PR; it stamps the date and
unblocks whatever waited on it). The claim on a resolved ticket goes dead by
itself and is swept, so there is nothing to release. If you stop without
finishing, `release` it.

## File

```sh
scripts/backlog new ipc my-slug --title "One line" --priority medium [--blocked "waits on X"]
```

Then fill in the template (what is wrong with evidence, what to do, what is
out of scope), and **link the entry from `docs/backlog/README.md`**
(`docs/scootbg/backlog/README.md` for scootbg, `docs/scootbar/backlog/README.md`
for scootbar). Check the claims against the code before writing them; entries
here have been wrong before.

## Edit

```sh
scripts/backlog set SLUG priority=high blocked=null status=research milestone=M2
```

Fields: `title`, `status` (`open|research|resolved`), `area`, `priority`
(`high|medium|low|research|null`), `blocked` (text, or `null` for nothing),
`milestone` (free text such as `M2` or `ongoing`).

`blocked` naming other entries by slug (`"module-api-and-clock, popups"`) is
what lets `resolve` unblock them automatically, so write blockers that way and
keep prose for real-world conditions.

## Resolve

```sh
scripts/backlog resolve SLUG [--token UUID]
```

Sets `status: resolved`, clears priority and blocked, **stamps
`resolved: "YYYY-MM-DD"`** (today; `--date` if it landed on another day), moves
the file to `resolved/<slug>-done.md` (`git mv` when tracked), forgets its
local claim record, and removes the slug from every other entry's `blocked` (clearing the
field when nothing else was there; prose is left alone and reported). Always
resolve through the script so the date is stamped. It refuses someone else's
live claim (checked against a fresh fetch of `main`) unless you pass that claim's
`--token` (or `--force`).

It does **not** write the resolution: add what landed, the evidence and the PR
to the entry's prose, then fix every link the move broke
(`grep -rn SLUG docs ROADMAP.md README.md`). Resolved entries keep their
diagnosis history; do not rewrite them.

## When did it land

```sh
scripts/backlog list --resolved              # newest first, with dates
scripts/backlog list --resolved --since 2026-09-01
```

The date is the stamped `resolved:` field. Entries archived before the field
existed have none, so the script falls back to git: the commit that moved the
file into `resolved/` (shown as `(git)`). In a shallow clone anything older
than the boundary reads `date unknown` rather than a wrong date;
`git fetch --unshallow` recovers it.

## Check

```sh
scripts/backlog check
python3 scripts/test_backlog.py              # the tool's own tests, if you changed it
```

`check` validates frontmatter, that `resolved/` and `status` agree, that `area`
matches the directory, and that each open entry is linked from a README or
`ROADMAP.md`. Run it before committing backlog changes. Exit status is 1 on
any problem.
