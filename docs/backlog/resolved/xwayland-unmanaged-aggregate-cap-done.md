---
title: "Multi-X-client aggregate unmanaged-window pressure: N clients × 128 menus per motion and per frame — RESOLVED (measured, don't build)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Multi-X-client aggregate unmanaged-window pressure — RESOLVED (don't build)

RESOLVED 2026-09-27 (verdict-only PR, no code change). Design winner,
cheapest-first per the ticket: **none of the three — don't build**. Measured
first, on the dev VM against a live XWayland server with K X clients × M
override-redirect menus each, nobody over the per-client cap: the residual
is linear at ~1.2 µs per window per pointer motion and ~0.4 µs per window
per frame (debug builds — upper bounds; release is strictly faster on this
straight-line lock-and-walk code), and the aggregate above ~250 windows is
unreachable anyway because XWayland itself dies there first (filed
separately: `docs/backlog/resolved/xwayland-server-death-many-unmanaged-done.md`).
By this project's own revealed bar — PR #260 left a 281 ms linear residual
at 5120 popups as acceptable — a 0.27 ms worst case at the maximum reachable
aggregate needs no fix. No README/protocols change: no user-facing behavior
changed.

**Correction (2026-09-27, same day):** "XWayland itself dies" was wrong.
scoot was disconnecting the server, because it held it to one app's
512-fd and 512-buffer bounds at 2 of each per X window. With its own
budget (`xwayland_budget.rs`), the ~250 ceiling below is gone. It is now
~2048 windows on the usual fd table. The per-window costs measured here
are unaffected. The "unreachable above ~250" leg of the verdict is not.
Extrapolating these debug figures linearly (not measured) gives ~2.5 ms
per motion at 2048 windows, which is still inside PR #260's 281 ms bar. If
it ever matters, the bbox pre-check in the residual below is the fix. See
`docs/backlog/resolved/xwayland-server-death-many-unmanaged-done.md`.

Filed 2026-09-26 from the PR #258 review. The per-X-client unmanaged cap
(`xwayland-unmanaged-pressure-cap.md`, resolved by #258) bounds each
client at 128 hit-test walks and draws — but the `x11_unmanaged` list is
global, so N X clients give N×128 per pointer motion and per frame with
nobody tripping a cap. Same class as `popup-aggregate-pressure-cap.md`
for popups; linear, not quadratic, and behind the session-owned X socket,
hence low. Serves daily-drivability (a runaway X app's menus should not
make the pointer lag) more than computer use.

## What was measured, and why the designs lost

A temporary `#[ignore]` probe (same shape as `xwayland/tests/bench.rs`'s
`x11_hot_path_cost`, deleted after measuring) mapped K real X clients × M
stacked override-redirect menus each through the live harness
(`SCOOT_REQUIRE_XWAYLAND=1`, `XW_PATH` on `PATH`), waited until every menu
was on the draw list *with a surface* (so the walks are the real ones, not
the no-surface early-`None`), then timed `x11_unmanaged_under` at a miss
point (the per-motion worst case: walks every tree) and a hit point, the
production `surface_under` around it, and `x11_unmanaged_frames` with an
all-overlapping region and a non-overlapping one (the reject loop alone).
Debug binary, dev VM, 5 runs each, best-of-5 reported:

| total | miss / motion | hit | `surface_under` miss | frames, all overlap | frames, none overlap |
| --- | --- | --- | --- | --- | --- |
| 2×5 = 10 | 12.8 µs | 1.4 µs | 13.6 µs | 12.3 µs | 3.2 µs |
| 1×128 = 128 | 158.4 µs | 1.3 µs | 120.8 µs¹ | 203.6 µs | 56.7 µs |
| 4×32 = 128 | 157.9 µs | 1.7 µs | 188.0 µs | 225.9 µs | 63.3 µs |
| 2×124 = 248 | 271.0 µs | 1.1 µs | 266.2 µs | —² | 91.3 µs |

¹ The 1×128 `surface_under` cell is noisy (min 49 µs against a 166 µs max;
shared-VM paging during that run) and reads below the unmanaged walk it
contains, which is impossible — the 4×32 and 248 cells (188/266 µs, at or
above the unmanaged walk) are the sound ones. Not load-bearing: the verdict
rests on the `x11_unmanaged_under` miss column, clean and linear in every
cell.

² The all-overlap frames cell sends a callback per window per call with
nobody pumping the X clients in between; at aggregate totals that flood
kills the X server (see below), so it ran only at small totals. It measures
necessary work (visible menus must be told), not residual — the reject loop
is the aggregate question and sends nothing.

Three things fall out:

- **Linear with a tiny constant.** 1.28, 1.23 and 1.09 µs per window per
  motion at 10/128/248 (debug). Per-client distribution is irrelevant:
  1×128 and 4×32 agree to the microsecond — only the global total matters,
  as the ticket predicted.
- **The per-frame walks already have their cheap reject.**
  `x11_unmanaged_elements` and `x11_unmanaged_frames` both skip a window on
  a `region.overlaps(rect_of)` integer test (two locks + math, ~0.4 µs);
  `x11_unmanaged_feedback` walks trees but only where feedback is queued.
  91 µs per frame per output at the maximum reachable aggregate (debug) is
  ~0.5% of a core at 60 fps. There is no per-frame scan left to index away.
- **The aggregate above ~250 is unreachable.** Three runs died the same
  way: 240/256 drawn and alive, then the final batch's maps kill the
  XWayland server (client round-trip fails with EPIPE/`Broken pipe`, the
  whole draw list drains, the cap zeroes). Scoot itself survives cleanly —
  Wayland clients unaffected, no panic, orderly drain — but no scene with
  more than ~250 override-redirect windows exists to be slow in. Filed as
  `docs/backlog/resolved/xwayland-server-death-many-unmanaged-done.md`; the
  exact count-vs-resource mechanism is XWayland's business, with one honest
  confound recorded there (the dev VM's disk was nearly full throughout).

**Option 1 (index/lookup) lost:** the per-frame side already rejects
cheaply, and a motion-path bbox pre-check would cut only a constant (~8× on
0.27 ms debug worst-case) without changing the complexity — while resting
on a subtle invariant (an X11 surface tree answering outside its `rect_of`
through a subsurface overhang, which only the session-owned XWayland
server's behavior rules out). Saving ~0.2 ms per motion in a pathological
scene is not worth pinning that invariant.

**Option 2 (global ceiling + shed policy) lost:** same reason it lost in
PR #260 — there is no stall for a ceiling to shed, and a ceiling would
invent refusal semantics (which client's menus drop? silent or loud?) for
traffic shaped exactly like legitimate use.

**Option 3 (don't build) won:** the worst measured case is 271 µs per
motion (debug upper bound; release only faster) at N = 248 with nobody
over a cap, ~13 µs at legitimate handful-of-menus counts, 91 µs per frame
per output. PR #260's bar for leaving a linear residual alone was 281 ms;
this is a thousand times smaller, behind a narrower socket, with a server
that bounds the aggregate itself.

## Evidence (record, not narrative)

- Base: `main` at `1591e74` (branch `xwayland-unmanaged-aggregate`;
  final diff is docs-only — the probe was temporary and deleted, no
  production code touched, so there is no before/after pair to report).
- Probe runs (dev VM, debug, `SCOOT_REQUIRE_XWAYLAND=1`, `XW_PATH` on
  `PATH`): `cargo test -p scoot --features xwayland --bin scoot
  unmanaged_aggregate_probe -- --ignored --nocapture --test-threads=1`
  with `SCOOT_AGG_CLIENTS`×`SCOOT_AGG_MENUS` = 2×5, 1×128, 4×32, 2×124.
  Best-of-5 medians in the table above (per-run spreads were not
  retained — only the one noisy cell's min/max, 49/166µs, survives in the
  PR discussion; the probe has since been deleted, so they cannot be
  re-derived).
- Existing tests unchanged and green on the same binary: the 5 hermetic
  `toplevel_cap::tests::x11_unmanaged` tests and the 3 live
  `xwayland::tests::unmanaged_cap` tests (`SCOOT_REQUIRE_XWAYLAND=1`).
- Release numbers: not measured — the release test-binary link
  (`lto = "fat"`, `codegen-units = 1`) does not fit the dev VM (4 GB RAM,
  no swap; rustc OOMs after hours), and the VM's disk spent the session at
  99–100%. The verdict uses debug figures explicitly as upper bounds.
- verdict-only PR: no `nextest`/`clippy` signal to re-derive beyond the
  unchanged tree (docs-only diff); `cargo fmt --check` clean.

## Honest residual (not this ticket)

- The per-motion walk stays O(N) with a ~1 µs/window debug constant. If X
  client counts ever make it matter, the sound fix is the bbox pre-check
  in `x11_unmanaged_under` (verified against the pinned Smithay fork rev
  `74edbf32`: `X11Surface::surface_under` delegates to
  `under_from_surface_tree`, whose per-surface `contains_point` first
  checks the surface's own rect — so a pre-check is behavior-preserving as
  long as X trees stay single-surface, which only the session-owned
  XWayland server controls — plus the `xdnd_active` early-`None`, which a
  rect check must not skip).
- The XWayland death at ~250 override-redirect windows is the real
  aggregate bound and the real availability story; see the follow-up
  ticket. Reproduce it with a freer disk before trusting the count.
