---
title: "Bring M4's idle memory and CPU back down"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M4"
---

# Bring M4's idle memory and CPU back down

Filed 2026-10-01. Serves **daily-drive** first: a bar that sits idle on a
laptop all day is judged by what it holds and what it wakes for, and the
ratchet exists to keep that from creeping. It serves **computer use** only
indirectly (the agent interface does not need the bar to be lean).

## The gap

The maintainer ruled on 2026-10-01 that M4's **binary size** is accepted and that
optimization effort goes to **usage** rather than disk space (user: "Keep the
button and exec inside the bar. Accept it. Aim for optimization in usage more
than pure disk space."; [lightest's Decisions](lightest.md#decisions)).
`button`, `push` and `exec` stay in the default features. That ruling is about
size and about where to spend effort; **it does not waive the idle memory
rows**, which regressed against M3 post-fix and `main` on the Asahi M2
([M4 on the Asahi M2](../README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2)),
scoot, scope clock, one pinned `scoot`, medians of each run:

| Row | `main` | #364 | #366 | #367 | growth per layer |
|---|---|---|---|---|---|
| Idle RSS (MiB) | 3.53 | 3.67 | 3.73 | 3.67 | +0.13 to +0.20 over `main` |
| Idle PSS (MiB) | 2.06 | 2.20 | 2.26 | 2.20 | +0.14 to +0.20 |
| Idle heap, `RssAnon` (MiB) | 0.44 | 0.45 | 0.45 | 0.45 | one or two 16 KiB pages |
| Idle CPU, 300 s (ms) | 0.93 to 0.95 | 0.96 | 0.98 | 1.04, 0.99, 1.10 | pooled `verdict()`: same (margin 0.128), single runs flag it |

Idle wakeups (2 a minute), threads, startup and the CPU of 240 workspace
switches are unchanged. **The final stack's re-measurement at the fixed heads
supersedes these where it differs** ([final stack](../README.md#m4-final-stack-the-fix-round-re-measured),
#367's tip `d0c4f35d1` against the A-B-A `main` runs): idle RSS 3.64 against
3.52 and 3.53 MiB (+0.11 to +0.13, inside the harness margin), **idle PSS 2.24
against 2.11 and 2.13 (+0.11 to +0.13, a regression by `compare`, on scoot and
sway)**, idle heap 0.42 against 0.42 and 0.44 (no growth that run), idle CPU
0.91 against 0.99 and 0.97 ms (not worse). So the row that stays flagged is
idle PSS, by about 0.1 MiB; RSS and heap flag in some runs and not others. The
acceptance bar below is judged on all of them.

**Where the PSS goes** (the PR 1 review's finding, measured): the idle PSS
increase is the executable mapping being larger. Its `smaps` `r-xp` mapping has
`Rss` **960 kB on `main` against 1088 kB at PR 1** (#364), +128 kB, which is
about the whole of the +0.13 to +0.14 MiB RSS/PSS step at that layer. The heap
is not where it goes (one or two 16 KiB pages). Which of the new code is
faulted in at startup, and why, is not yet known: that is what the levers
below measure.

## What to do

Measure, in this order, and keep only what moves a row; **none of these is an
assumption that it will**:

1. **`[profile.release.package.scootbar] opt-level = "s"`.** Already measured
   once on #367 (the README's experiment): `.text` -16.1%, idle RSS/PSS to
   within the margin of `main`'s, the closure inside the size margin, idle CPU
   a pooled tie (margin 0.265; the median was +0.14 ms on three runs).
   **Redraw cost is not measured**: the harness has no redraw-heavy row, so
   this needs one first (a text-heavy bar redrawn at its worst rate, or
   `scootbar msg reload` in a loop as the appearance hardware test does)
   before the profile is judged. A change of a codegen profile for one crate
   is a one-line trade; the evidence it needs is on the redraw side.
2. **Share the three `toml` table shapes** (`BTreeMap<String, ButtonFile>`,
   `PushFile`, `ExecFile`). An estimate, from `nm --size-sort`: about 33 to
   36 KB of `.text`. It rewrites `config/custom.rs`'s per-kind unknown-key
   refusal, so it must keep each kind's dotted-name error. Measure what the
   idle RSS/PSS gains, not the file size.
3. **Cold-path outlining**: keep the `toml`, `exec` and agent code out of the
   pages startup and the idle tick touch (`#[cold]`, `#[inline(never)]`, a
   linker section ordering if cheap), so an unconfigured bar does not fault
   them in. The check is the `r-xp` `Rss` in `smaps` after settling, against
   `main`'s 960 kB, before and after.
4. **Avoid page-faulting code that idle never runs**: find what startup and
   the idle tick actually touch (`/proc/PID/smaps` per-mapping `Rss`,
   `/proc/PID/stat` minor faults at startup, before and after settling), and
   whether an unconfigured bar still builds or deserializes anything for the
   three custom-module tables. Only code that those show is touched is worth
   moving.

The acceptance bar: with the M4 modules compiled in but **unconfigured**, no
row worse than `main`'s by more than the harness's noise margin on idle RSS,
idle PSS, idle heap and idle CPU, at `--scope clock`, scoot and sway, the
harness's defaults, a pooled `verdict()` over at least three runs for the CPU
row, and the same machine (the Asahi M2), with `main` re-run beside it so
drift shows.

## Not in this ticket

- **Binary size.** Accepted (lightest's Decisions, 2026-10-01); a change here
  that shrinks it is welcome but is not the goal and is not a reason to take a
  slower or less safe path.
- Removing `button`, `push` or `exec` from the default features: the
  maintainer kept them in the bar.
- What *using* the modules costs (a configured `exec` module's wakeups): the
  dev VM's table in
  [exec-push-button-modules-done](resolved/exec-push-button-modules-done.md#the-ratchet)
  is the record; that is its own measurement.
