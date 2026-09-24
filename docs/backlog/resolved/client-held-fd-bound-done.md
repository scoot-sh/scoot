---
title: "A client could make scoot hold ~900 fds that no per-client cap counted (syncobj points on destroyed timelines; dmabuf params adds) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Client-held fds no cap counted — RESOLVED

RESOLVED 2026-09-24 (PR #PRNUM). Both paths the review of PR #233 measured
are now counted per client, under the same cap-plus-pressure-grace model as
buffers and pools. The offender is refused at its cap, and fd pressure's
creation guards can pick it.

## What changed

- **Pending dma-buf planes** (`dmabuf/pending_planes.rs`, every tier with
  the dmabuf global). Claimed on `zwp_linux_buffer_params_v1.add`, before
  delegation, per params object (`ObjectId`, serial included) and summed per
  client. Released when the params object is consumed (`create` /
  `create_immed`, before delegation: Smithay drains the planes into the
  `Dmabuf` there whatever the import's outcome) or destroyed (the blanket
  `destroyed` hook, which also covers disconnect and kill). An `add`
  Smithay itself refuses was claimed, and is released by the dead client's
  params destroy, so no phantom outlives the connection. Cap **32** per
  client (8 whole four-plane buffers; every client this project knows
  sends one buffer's planes and creates it at once, so at most 4), grace
  **8** under fd pressure. Past either, the client is disconnected with
  `wl_display.error(no_memory)` (shared with the acquire-wait bound in
  `no_memory.rs`; the params interface has no fitting error).
- **Retained syncobj timelines** (`drm_syncobj/retained.rs`, GPU scanout
  tier). The 128 cap now counts the timeline *fds* scoot holds, not live
  objects. Smithay's `DrmTimelineInner` owns exactly the `OwnedFd` from the
  request, unchanged, so the fd closing *is* the timeline being freed. A
  ledger records each import's fd number against its client, before
  delegation. It forgets a record when the same number reaches scoot again
  through `import_timeline`, `add` or `create_pool` (the kernel reuses only
  closed numbers), or when a sweep finds it closed (`fcntl(F_GETFD)`) or no
  longer a syncobj (`readlink /proc/self/fd/N` is not
  `anon_inode:syncobj_file`, verified on the dev VM's 6.18 kernel). A
  refusal is only ever decided on a fresh sweep: an import at the cap sweeps
  and is refused only if more than 112 are still open. A sweep that admits
  leaves at least 16 imports before the next, which keeps an attacker
  hovering at the cap from buying a sweep per import. Under fd pressure the
  grace is 32, checked the same way, with up to 16 imports of slack after a
  sweep that admits. The count is released when the fd closes, whatever
  held it: pending points, committed points, commits queued behind a
  blocked one, the renderer's `Buffer`, or frames in flight.

## Deviation from the brief, and why

The brief preferred a scoot-side `Arc`/`Weak` or a wrapper in scoot's own
bookkeeping. Neither can see the retention. `DrmTimeline` wraps a
`pub(super)` `Arc`, so scoot cannot hold a `Weak` or read the count. scoot
stores no sync points of its own either: they live in Smithay's pending
and current cached state, the per-surface queue of commits waiting behind a
blocked one, and the renderer's `Buffer`. A mirror of those (a scoot
`Cacheable` riding the same commit ids) was considered, and it would still
miss a `Buffer` kept for a re-committed identical `wl_buffer`, which is
per-surface and uncounted: the same hole again. The fd is the one thing
that is freed exactly when the timeline is, and scoot can observe it
without touching the Smithay fork.

**On the alternative** (fd pressure attributing fds per client and
killing the holder at accept time): not built. The fd-pressure model is
unchanged: kills still land only through a counted creation past a grace.
Once the uncounted paths are counted that model can pick the holder, and an
accept-time kill of the biggest holder would change what pressure does to
living clients. The fd ledger here is the natural base for per-client fd
attribution if it is wanted, extended to pool and plane fds. That is the
follow-up below.

## Evidence

All on the dev VM (kernel 6.18, virtio-gpu, `RLIMIT_NOFILE` 1024/524288),
release builds. Before is `618b5dc` built by the same command
(`CARGO_BUILD_JOBS=1 cargo build --release -p scoot --features
gpu-scanout`); the PR #235 binary (`e0e5a55`, same code tree) was used for
the first before-runs and gave the same shape. After is `559306d`, the
final code tree of this change (binary sha256 `c77ee353…`). Raw output:
`~/evidence/cfb/runs/final/*.txt` (earlier runs at `e872826` in
`~/evidence/cfb/runs/`), probe source `~/evidence/cfb/probe/src/main.rs`,
runners `~/evidence/cfb/{run,legit,bench,gate}.sh`.

| Shape | Tier | Before | After |
|---|---|---|---|
| 220 params x 4 adds, never created | `--headless` (pixman) | scoot at 899 fds; `wayland-info` shed (0 globals); `scootctl version` refused (pressure); honest dma-buf client reset; offender connected after 12 s | offender disconnected at params 9 (`no_memory`, "32 dmabuf planes"); scoot at 18 fds; `wayland-info` 38 globals; `scootctl` served; honest client 145 commits, frame callbacks p95 21.5 ms |
| 440 surfaces, pending points on 2 destroyed timelines each | `--tty --renderer gles` | 927 fds (880 `syncobj_file`); newcomers shed; `scootctl` refused; honest explicit-sync client reset; offender connected | offender refused at surface 65 (`invalid_timeline`, "still holds 128"); 46 fds; newcomer 39 globals; `scootctl` served; honest explicit-sync client 132 commits, 0 reuse timeouts |
| the same, points committed with one dumb dma-buf | `--tty --renderer gles` | 929 fds; the same | refused at surface 65; 46 fds; the same |

Real clients against the after binary, 5 s each, both tiers:
`es2gears_wayland`, `eglgears_wayland`, `vkcube` (llvmpipe), `mpv --vo=gpu`
all alive with one window, no refusal or protocol error logged, scoot back
to baseline fds; an explicit-sync client (card0 dumb buffers, one acquire
and three release timelines) 177 commits, 0 reuse timeouts on the GPU tier.
None of those real clients makes a dma-buf or imports a timeline on this VM
(GBM is refused on its render node), so the dma-buf and explicit-sync
paths were exercised by the probe clients above and by the harness.

Tests (fail-first against `618b5dc`, `~/evidence/cfb/failfirst-main-618b5dc.out`):
`pending_planes_are_bounded_per_client` (main: the client survived),
`pending_points_on_destroyed_timelines_count_against_the_bound`,
`committed_points_on_destroyed_timelines_count_against_the_bound` (main:
survived), `timelines_held_by_points_stop_counting_when_the_points_go`
(main: 1 counted, 81 held). Plus pairing tests (consume, destroy,
disconnect, Smithay-refused add, per-client isolation) that assert the
count and the real fds (tagged memfds counted in `/proc/self/fd`) agree,
the ledger's decisions in isolation, and the two pressure verdicts.

Cost: pending-plane bookkeeping 16 ns per plane (release microbench). The
end-to-end add path (400 k params x 4 adds + destroy, headless, scoot CPU
jiffies, 3 alternating rounds, `~/evidence/cfb/bench-churn-params-400k-559306d.txt`) went from 81/79/78 to 85/84/85, about 40 ns
per `add`; real clients do one buffer's adds per allocation, not per frame.
A 128-record sweep costs 100 us (786 ns per record, real syncobj fds), at
most once per 16 imports.

## Still open

`docs/backlog/core/buffer-fds-past-their-object.md`: a buffer committed to
a surface keeps its fd after its `wl_buffer` (and pool) objects are
destroyed, one per surface, uncounted (harness: 200 surfaces, 200 fds, 0
counted); and the buffer count weighs a multi-plane dma-buf as one fd.
Found while doing this; not part of this ticket's scope.

## Original entry

Filed 2026-09-24 from the review of PR #233 (explicit sync). It serves
**daily-drive**: a client-triggerable resource exhaustion that sheds other
clients.

## What is wrong

`fd_pressure.rs` is sized on the claim that every fd a client can make scoot
hold is counted by some per-client cap (buffers, pools, timelines, waits),
so one connection cannot exhaust the table alone and the pressure guard's
kill lands on a contributor. Review found two paths that break it.

1. **Syncobj points on destroyed timelines** (GPU scanout tier, where
   explicit sync is offered). An imported timeline keeps the client's
   syncobj fd open in scoot, and every `DrmSyncPoint` on it holds the
   timeline's `Arc`. A point set on a surface outlives the destruction of
   the timeline object (the protocol requires that), but
   `drm_syncobj::forget_destroyed` releases the 128-timeline count on
   destroy. So "import, set points on a fresh surface, destroy the
   timeline", repeated, keeps one fd per surface with zero live timelines
   counted.
2. **dmabuf `params` adds that are never created** (every tier with the
   dmabuf global). Each `zwp_linux_buffer_params_v1.add` hands scoot a
   plane fd, which the params object keeps until it is used or destroyed.
   A client that creates params objects, adds planes and never calls
   `create`/`create_immed` keeps the fds, and no cap counts params
   objects or their planes (`wl_buffers.rs` counts only buffers that
   were created).

## Measured (reviewer, dev VM, 1024-fd table)

- Path 1: 440 surfaces with pending points on destroyed timelines put
  **927 fds** in scoot with **0 live timelines**.
- Path 2: 220 params objects with 4 adds each, never created, put the same
  927 fds in scoot.
- In both, new clients were shed at accept (the pressure reserve), and
  **the offender was not killed**: none of its creations is past a counted
  grace, so fd pressure's creation guard cannot pick it. The kill is meant
  to land on a contributor; here the contributor holds uncounted fds.

## What to do (not done in PR #233)

Count what actually retains fds, per client:

- params planes: claim on `add`, release on params destroy or consume, cap
  and grace like buffers;
- retained syncobj timelines: claim on import and release when the
  `DrmTimeline` is really freed, not when its object is destroyed. That
  needs a drop hook scoot can see, for example counting fds through a
  wrapper, or another Drop-side change in the Smithay fork
  ([repin ticket](./smithay-fork-repin.md));
- alternatively, make fd pressure attribute fds per client (e.g. from the
  objects each client holds) so the kill can find the real holder.

Whichever it is, re-measure both paths live, and fix `fd_pressure.rs`'s
module doc, which now states the gap.
