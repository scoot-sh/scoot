---
name: update-docs
description: Rewrite or review user-facing documentation (README, guides, runbooks) so it serves readers rather than reviewers. Use when a doc has grown long, mixes audiences, reads defensively, or when asked to make docs concise, helpful, or user-aimed. Also use before writing a new user-facing doc, to avoid the same drift.
---

# Writing docs for readers, not reviewers

Engineering docs written alongside the code they describe drift toward being
a **conformance record**: every feature arrives with evidence it was really
done, every limitation with a defense of why it is deliberate. That is
writing aimed at a reviewer who might accuse you of stubbing something.

Readers do not arrive suspicious. They arrive with a question.

Apply this when rewriting a doc, and when writing a new one.

## 1. Count the audiences

A file serving three audiences serves none. Name them explicitly before
editing. The usual three for a systems project:

- **End users** — can I run this? will it break? how do I configure it?
- **Integrators** — will my bar/client/toolkit work against it?
- **Automation authors** — what exactly will the API refuse, and why?

Then decide which audience owns the file, and move the rest out. For a
README the answer is almost always: users own it, everyone else gets a link.
A good split is one file per audience, not one file with three acts.

## 2. Defensive constructions are a tell

Search the draft for these and cut them:

- "deliberate, not an oversight"
- "that is a refusal, not a stub"
- "measured and closed as unreachable rather than stubbed"
- "this is intentional"
- "not a limitation, a design choice"

**In a doc for users, stating the behavior plainly *is* the confidence.**
"Single output only" reads as honest. "Single output only — this is
deliberate, not an oversight" reads as anxious, and invites the doubt it is
trying to pre-empt. If a limit genuinely needs justifying, one clause is the
budget, and it belongs next to the limit, not as its own paragraph.

## 3. Evidence belongs in the PR, not the doc

Cut from user-facing docs, keep in the PR body, roadmap, or design record:

- **Measurements** — benchmark numbers, timings, settle times, sampled
  counters. Evidence for a reviewer, noise for a reader.
- **Resource bounds** — fd ceilings, buffer caps, per-client limits. Users
  cannot hit them; integrators and automation authors need them. Move, do
  not delete.
- **Internal references** — links to resolved tickets, backlog entries, PR
  archaeology. This is the loudest possible tell that the reader is assumed
  to be working *on* the project rather than *with* it. A user-facing doc
  should contain approximately zero of them.

## 4. Prose that enumerates should be a table

A long sentence-chain listing thirty things is unreadable at any quality of
prose. Protocols, flags, keybindings, supported formats, feature states —
all tables. Columns earn their place: name, version/value, state, link.

The rule of thumb: if a paragraph contains more than about four
comma-separated items of the same kind, it wanted to be a table.

## 5. Relocate, never drop

"Make it concise" is almost never "delete information". It is "move detail
to a reference and link to it". The distinction matters because a project
often has a standing rule that every option/flag/command stay documented —
check for one before cutting, and treat losing a documented option as a
regression rather than a cleanup.

**Verify the relocation mechanically.** Before finishing, diff the *set* of
documented options, flags, keys and commands between the old file and the
new file-plus-references. Report that check. It is the single way this kind
of rewrite quietly causes harm.

## 6. What is missing matters as much as what is bloated

Long docs are usually missing the things a reader wants first:

- **A picture, if the thing makes a visual claim.** A layout, a UI, a
  diagram-shaped idea. Generate a real one from the actual software rather
  than mocking it up; a bad screenshot is worse than none.
- **The credibility signal, up top.** "Daily-driven on real hardware" or
  "in production at N" is usually buried mid-document. It is the single
  strongest line in the file. Move it up.
- **Why this over the obvious alternative.** Docs that say "in the shape of
  X" rarely say why someone already using X would switch. Answer it
  explicitly.
- **A "not yet" list, near the top.** Three to five lines. Readers trust a
  doc that tells them what it cannot do, and it prevents the support
  question you would otherwise answer forever.

## 7. Check claims against HEAD, not against memory

Every factual claim in a doc rots. Before shipping a rewrite, verify the
limitations list, version numbers, flag names and defaults against the
current code — not against what was true when the doc was written, and not
against what the task brief said. A rewrite that faithfully preserves a
stale claim has laundered it into looking freshly checked.

This is especially true for "not yet" lists, which are written once and
then quietly become false as the feature lands.

## Checklist

- [ ] Audiences named; file has exactly one owner
- [ ] Defensive constructions removed
- [ ] Measurements, resource bounds and internal ticket links relocated
- [ ] Enumerating prose converted to tables
- [ ] Documented options/flags/commands diffed old vs new — nothing lost
- [ ] Screenshot or diagram if the claim is visual
- [ ] Credibility signal and "why over the alternative" near the top
- [ ] "Not yet" list present and verified against HEAD
- [ ] Every internal link resolves
