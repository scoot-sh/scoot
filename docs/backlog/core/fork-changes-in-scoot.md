---
title: "Investigate: which of the carried Smithay and wayland-rs fork changes could live in scoot instead"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Which fork changes could live in scoot instead?

Filed 2026-09-29. Serves **daily-drive** through less maintenance debt: every
carried commit is one to rebase on each Smithay bump, one more thing
`CLAUDE.md`'s "verify against the pinned source" has to be checked against, and
one more reason [the repin](smithay-fork-repin.md) is blocked. This is an
**investigation**, not a decision to move anything.

Start from [the decision audit](resolved/fork-decisions-audit-done.md): it classifies how well
each fork's alternatives were recorded, and whatever it finds thin goes first here.

## What is carried (`docs/forks.md` is the source of truth)

`scoot-sh/smithay` (26 commits on upstream `0ff00983`, tip `7ab72d53`) and
`scoot-sh/wayland-rs` (2 commits on 0.3.17), grouped:

| Group | What it does | Route worth checking |
|---|---|---|
| syncobj `Drop` | stops each timeline import leaking a kernel handle | an RAII guard in scoot that closes the handle itself, if the handle is reachable from outside |
| XWayland selection transfers (about a dozen) | INCR pacing, bounded reads, per-client and per-owner transfer bounds, timed sweeps, orphan detection | in-tree module, or bounds enforced from scoot's `XwmHandler` side |
| XWM hooks (`selection_owner`, `selection_generation`, `allow_drag`, `set_selection_transfer_timeout`) | expose state scoot's gates need | a wrapper tracking the same state from events scoot already sees |
| XDND (seven) | proxy remap flush, ending offers to dead X targets, entering another client's window without waiting for types | in-tree module, or direct `x11rb` requests beside the XWM |
| pixman `Repeat::Pad` | clamp the source image at its edge when upscaling | a scoot-side workaround (a custom render element, pre-padding, avoiding the bilinear tap), if the renderer exposes any seam |
| cached buffer scale and transform on commit | apply them without a new buffer | scoot's commit handler doing what the renderer does not |
| XSETTINGS flush | scale reaches X toolkits | a flush from scoot after its own write, if the connection is reachable |
| `set_commits_allowed` | unused; drop at the next rebase | nothing: just delete it |
| `UnderlyingStorage::Dmabuf` | a compositor-owned dma-buf (the drawn cursor) can ride a plane | **already evaluated** in `docs/forks.md` (`7ab72d53`): a phantom-client `wl_buffer` and an exporter wrapper were ruled out |
| wayland-rs fd-queue cap | disconnect a client leaving too many unclaimed fds | **already evaluated**: scoot-side attribution, a kill heuristic and a socket proxy were ruled out (`wayland-backend-fd-queue-done.md`); reopen only with a new idea |

## Method, per carried commit

1. What does it need that Smithay does not expose (private state, an internal
   loop, a hook that runs at a moment scoot never sees)?
2. Can a public API, a handler trait or a wrapper type express it? If yes, what
   does the scoot-side version cost in lines, allocations and hot-path work?
3. If it needs Smithay's internals: is **copying the MIT-licensed module into
   scoot** (keeping the notice) cheaper than carrying a patch? A copy has no
   fork to rebase, but it diverges and becomes scoot's to maintain; measure its
   size and how coupled it is to the rest of Smithay before judging.
4. What happens to the commit's fail-first tests and evidence (`~/evidence/xw4/`,
   the drop and drag suites)? They must move with the behavior, unchanged in
   what they assert.
5. Verdict: **moves to scoot** (with the route), **stays a fork** (with the
   reason), or **delete**.

Verify every claim against the **pinned fork's** source
(`~/.cargo/git/checkouts/smithay-*/7ab72d5`), as `CLAUDE.md` requires, not from
memory of Smithay.

## Constraints

- **Dependency fixes still go in scoot-sh forks and nothing goes upstream from
  here** (`CLAUDE.md`). This asks whether scoot itself can carry a behavior, not
  whether to open PRs.
- `scoot-core` stays platform-independent: nothing here belongs in it.
- The hot-path rules apply to any scoot-side version: no allocation per event.

## Output

A "could it live in scoot?" column and a verdict per commit added to
`docs/forks.md`, and one follow-up ticket per verdict that says "moves", each with
its plan and its tests. If most commits move, [the repin](smithay-fork-repin.md)
shrinks to a plain upstream pin and this entry's follow-ups replace it.

## Done when

Every carried commit has a verdict with evidence from the pinned source, and the
follow-up tickets exist.
