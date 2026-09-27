---
title: "XWayland dies with ~250 override-redirect windows mapped, draining every X menu at once"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland dies with ~250 override-redirect windows mapped

Filed 2026-09-27 from measuring
`core/xwayland-unmanaged-aggregate-cap.md` (resolved don't-build in the
same PR). While building the aggregate scene that ticket asked about (K X
clients × 128 override-redirect menus, nobody over scoot's per-client cap),
the XWayland server died instead — three times, the same place. Serves
**daily-drive** (a runaway X app's menu storm should not take down every X
app's windows) more than computer use; behind the session-owned X socket,
hence low.

## What happens

Dev VM, XWayland 24.1.13 (the `~/xw` nix store path), real X clients over
`x11rb` mapping stacked 120×90 background-pixel override-redirect windows
in batches with a settle between, scoot's live headless harness:

- Batches up to 240/256 total: every map draws promptly, server round-trips
  fine.
- The final batch (240 → 256): the server dies mid-batch. The test X
  client's next round-trip fails with EPIPE (`Broken pipe`, os error 32);
  scoot's whole `x11_unmanaged` list drains and the unmanaged cap zeroes;
  scoot itself survives cleanly (Wayland clients unaffected, `xwm` still
  attached, no panic — the orderly server-death path).
- A 512-window synchronous burst died the same way (zero maps ever
  processed: the server died mid-burst before the harness pumped).
- 2×124 = 248 total mapped, drew, timed and stayed alive end to end.

So the threshold is somewhere in (240, 256], and it is the server process
going away — not a scoot refusal (caps untouched: 128/client max, and the
cap drains rather than trips) and not the harness (the flood-phase
callback storm that kills servers in benchmarks was explicitly kept out of
these runs after it claimed one).

## Confounds to clear before trusting the count

- The dev VM's root disk sat at 99–100% through all of these runs
  (a full `/var/cargo-target` plus session scratch; ~150–250 MB free).
  XWayland's per-window backing is `/dev/shm` (tmpfs, 1.9 GB, empty —
  checked), so disk pressure *should* be irrelevant, but reproduce with a
  freer disk before trusting the exact threshold.
- RAM was free throughout (3+ GB); not OOM by the numbers, but nobody
  watched the XWayland process's own RSS across the storm.
- Whether it is count-at-rest (~250 live OR windows) or burst rate was not
  separated: the batched runs still map 16 per batch with one settle
  between. Map 240, pause, then add one window at a time with a round-trip
  check each to pin the fatal map.

## What a fix would need to decide

Whether the death is an XWayland bug worth carrying a fork workaround for
(`docs/forks.md` — the project already carries a scoot-sh Smithay fork,
but XWayland itself is nixpkgs-pinned, a different kind of carry), a
resource limit worth documenting, or something scoot's spawn should harden
against (restart? refuse maps past a scoot-side total?). Do not implement
here.
