---
title: "Audit: did we reach for a fork before exhausting scoot-side options? Check each fork's decision record"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Audit the fork decisions

Filed 2026-09-29, from the maintainer's concern that once forking was available it may
have become the easy way out. Serves **daily-drive** through less maintenance
debt, and serves the project's own standard: a fork is a cost that has to earn its
place with evidence.

This is a **look backward at how each decision was made**. It is separate from
[fork-changes-in-scoot](fork-changes-in-scoot.md), which looks forward at which
carried commits could move into scoot. The audit's output feeds that
investigation: any commit whose record is thin becomes a priority candidate there.

## Context, so the audit is fair

The fork policy came from the maintainer (2026-09-24: "I would rather fork for now
and maintain a list of forks", `CLAUDE.md`), so forking was an accepted interim
choice, not a lapse. The question is narrower: for each carried commit, was the
scoot-side alternative actually weighed, and is the record of that good enough
to trust?

## Method

For each fork (`scoot-sh/smithay`, `scoot-sh/wayland-rs`) and each group of carried
commits in `docs/forks.md`, read its resolved record and classify the evidence:

1. **Alternatives listed and ruled out with measurement or a concrete
   constraint.** Fine; note where the evidence lives.
2. **Alternatives mentioned and dismissed without evidence.** Weak; re-derive.
3. **No alternatives recorded.** A gap; treat as unproven.
4. **A hard constraint forces the fork** (state private to Smithay, a hook that
   runs where scoot cannot). Check that the constraint is real by reading the
   **pinned fork's** source (`~/.cargo/git/checkouts/smithay-*/035d447`), not from
   memory.

Also ask, of each: was there a **cheaper scoot-side mitigation** that was skipped
because the fork was simpler? Was the fork's cost (rebase burden, `outputHashes`,
verify-against-fork rule) counted at the time?

## Starting notes (from a keyword search only: not findings)

- `wayland-backend-fd-queue-done.md` and `docs/forks.md` say scoot-side alternatives
  (per-client attribution, a kill heuristic, a socket proxy) were evaluated and ruled
  out, with evidence on the dev VM. Likely class 1; confirm.
- `syncobj-handle-leak-done.md` has a section "Why it could not be fixed in scoot
  alone". Read whether the reasoning holds against the source.
- `buffer-scale-without-new-buffer-done.md` mentions a workaround that already
  existed for scootbg. Read what was weighed against the fork.
- The XWayland selection and XDND records, and the pixman `Repeat::Pad` record, showed
  no explicit alternatives section in that search. Where to look hardest.

## Output

- A short findings table (per commit group: class 1 to 4, where the evidence is,
  and a verdict on whether it needs re-examining) in `docs/forks.md`.
- Every class 2 or 3 item added to the priority list of
  [fork-changes-in-scoot](fork-changes-in-scoot.md).
- **A process fix**: `docs/forks.md` and the fork rule in `CLAUDE.md` gain a
  requirement that any new fork or carried commit states the scoot-side alternatives
  considered and why each was rejected, so the next fork cannot skip it.

## Done when

Every carried commit group has a classified record, the gaps are queued in the
investigation, and the process rule is in place.
