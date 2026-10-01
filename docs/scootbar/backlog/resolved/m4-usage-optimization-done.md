---
title: "Bring M4's idle memory and CPU back down"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M4"
resolved: "2026-10-01"
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
than pure disk space."; [lightest's Decisions](../lightest.md#decisions)).
`button`, `push` and `exec` stay in the default features. That ruling is about
size and about where to spend effort; **it does not waive the idle memory
rows**, which regressed against M3 post-fix and `main` on the Asahi M2
([M4 on the Asahi M2](../../README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2)),
scoot, scope clock, one pinned `scoot`, medians of each run:

| Row | `main` | #364 | #366 | #367 | growth per layer |
|---|---|---|---|---|---|
| Idle RSS (MiB) | 3.53 | 3.67 | 3.73 | 3.67 | +0.13 to +0.20 over `main` |
| Idle PSS (MiB) | 2.06 | 2.20 | 2.26 | 2.20 | +0.14 to +0.20 |
| Idle heap, `RssAnon` (MiB) | 0.44 | 0.45 | 0.45 | 0.45 | one or two 16 KiB pages |
| Idle CPU, 300 s (ms) | 0.93 to 0.95 | 0.96 | 0.98 | 1.04, 0.99, 1.10 | pooled `verdict()`: same (margin 0.128), single runs flag it |

Idle wakeups (2 a minute), threads, startup and the CPU of 240 workspace
switches are unchanged. **The final stack's re-measurement at the fixed heads
supersedes these where it differs** ([final stack](../../README.md#m4-final-stack-the-fix-round-re-measured),
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
  [exec-push-button-modules-done](exec-push-button-modules-done.md#the-ratchet)
  is the record; that is its own measurement.

## Resolution (2026-10-01)

Done in [PR #372](https://github.com/scoot-sh/scoot/pull/372):
the numbers, the attribution and every lever tried are in
[the README's "M4 usage optimization"](../../README.md#m4-usage-optimization); the raw
runs are `bench/m4-usage-*` ([index](../../bench/README.md)), every `compare` and the
pooled verdicts [`m4-usage-compares.md`](../../bench/m4-usage-compares.md), the `smaps`
and `pagemap` evidence [`m4-usage-attribution`](../../bench/m4-usage-attribution/README.md).

- **Where it went.** All of the idle PSS step was file pages of the bar's own
  executable (`r-xp` `Rss` 960 kB on `main`, 1152 kB at the tip); heap, stack, faults
  and the executed code (231 against 232 functions) were the same. The ~240 functions
  an idle bar runs sit in 14 of 17 64 KiB windows of `.text`, and the kernel maps a
  file's pages 64 KiB at a time.
- **What landed.** A hot-text order file (`crates/scootbar/orderfile/hot-text.ld`, a GNU
  ld `--section-ordering-file` of globs) that a `build.rs` hands the linker when a
  trial link says it takes it, the tool that makes and checks it
  (`scripts/scootbar-orderfile/`), and `scripts/scootbar-bench/redraw.py`, the redraw
  benchmark the ticket asked for first. `r-xp` `Rss` 640 kB; idle PSS 1.79 to 1.81 MiB
  against `main`'s 2.04 to 2.06 and the stack's 2.25 on scoot; idle RSS 3.27 to 3.28
  against 3.50 to 3.53 and 3.72; the same binary size, redraw cost, idle CPU, wakeups
  and startup.
- **Acceptance bar: met.** Idle RSS, PSS and peak better than `main`'s, heap and idle CPU
  the same (pooled `verdict()` over three runs a side, scoot and sway, `main` re-run
  beside it), wakeups 2 a minute, no other row regressed. The **size row** is
  +19.0% and was not part of the bar (the ruling above); it is neither waived nor moved.
- **Levers dropped, measured.** `opt-level = "s"` for scootbar: `.text` -16% but +31%
  CPU per redraw; `opt-level = 2`: +18%; per-dependency size levels, `codegen-units = 16`
  and `--sort-section=name`: no gain; lld and GNU ld with exact symbol names: the names
  differ between a `cargo` build and the Nix package (182 of 235 hot names), so only a
  glob file carries. Not done: sharing the toml table shapes and `#[cold]` hints
  (cold code costs no page once the hot code is together), `madvise` (`unsafe`).
- **Not taken, measured:** no-PIE (-0.09 MiB PSS, gives up ASLR) and DT_RELR (-0.12
  MiB, needs glibc 2.36); a decision for the maintainer.
- **Open questions.** The profile was taken on aarch64 only; x86_64 should gain by the same
  mechanism, unmeasured. The file decays as the code changes (a function no longer listed
  goes wherever the linker puts it); `scripts/scootbar-orderfile/orderfile.py check`
  says by how much and regenerating it is one command
  ([testing.md](../../testing.md#the-hot-text-order-file)).
