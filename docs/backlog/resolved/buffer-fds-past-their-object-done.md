---
title: "Buffers kept their fds past their own wl_buffer object, uncounted; multi-plane dma-bufs counted as one — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Buffer fds retained past their object — RESOLVED

RESOLVED 2026-09-24 (PR #239). Every fd a client hands scoot that scoot
keeps is now counted per client, for exactly as long as it is really open,
whatever holds it: a per-client fd ledger (`client_fds.rs`), generalised
from PR #236's syncobj timeline ledger. One bound (512 fds per client) and
one pressure grace (128) replace the per-kind object counts as fd pressure's
attribution. The live-buffer, live-pool, pending-plane and timeline caps
stay.

## What changed

- **One ledger for every kept client fd** (`client_fds.rs`, replacing
  `drm_syncobj/retained.rs`). `wl_shm.create_pool`, `zwp_linux_buffer_params_v1.add`
  and `import_timeline` each record the request's fd by number against its
  client, before delegation. A record dies when the number arrives again
  (the kernel reuses only closed numbers) or when a sweep finds it closed.
  Pool and plane records carry the file's `fstat` identity from arrival, so
  a number reused by one of scoot's own fds (a new client's socket, an
  eventfd) reads dead; timeline records keep the syncobj link-name check,
  since every syncobj shares one anonymous inode. Refusals are decided only
  on a fresh sweep, with PR #236's 16-arrival margin keeping sweeps off the
  per-request path. A pool counts once however many buffers share it; a
  plane counts once per `add`, so a four-plane buffer is four.
- **The bounds.** 512 fds per client, every kind together (refused with the
  arriving request's own error: `invalid_stride` on `wl_shm`,
  `wl_display.no_memory` for an `add`, `invalid_timeline`). The timeline
  cap (128) is now a per-kind cap inside the same ledger. Under fd pressure,
  one 128-fd grace on the client's whole fd count replaces the 128-buffer,
  64-pool, 8-plane and 32-timeline graces. Buffer creation is no longer a
  pressure site: it hands scoot no fd. The acquire-wait bound (scoot's own
  eventfds) keeps its own cap and grace.
- **Renderer copies** (`dmabuf/renderer_copies.rs`), found while doing this.
  On the dev VM's GLES renderer (Mesa llvmpipe via `kms_swrast`) every
  imported plane costs scoot a second fd, a duplicate the driver keeps for
  as long as the texture cache holds the import. A three-plane `YU12`
  buffer is six fds, a single-plane `XR24` one two, pixman none. The main
  run below shows `main` already held 1200 dma-buf fds for 200 `YU12`
  buffers: 600 planes and 600 copies. scoot now measures the copies once
  per session, on the first cleanly measurable GLES import: the fds in the process
  naming the imported file just after the import, less those just before.
  Every bound reads the weighted sum, and a plane is *admitted* at its full
  weight: 1, plus the learned copies per GLES backend (plus 1 per backend
  until the session has learned the number). An import only ever lowers a
  plane's weight to what was learned. The first version of this PR added
  the copies at import instead, after the bound had been checked, and
  review took a client to **540** fds against 512 that way: 80 committed
  `YU12` buffers, 30 planes added (each fitting at weight 1), then all
  imported. `renderer_copies_are_inside_the_bound_when_a_plane_is_admitted`
  pins that shape. It failed on the import-time version ("all 30 adds were
  admitted, and after the imports the client holds 540",
  `~/evidence/bfl/overshoot-failfirst-0111f57-plus-test.txt`) and passes now.
  The reserve test (`fd_pressure/tests.rs`) now drives the ledger's own
  admission rule with every kind and weight rather than adding constants. Two ways the measurement could be steered
  were caught before the PR opened (advisor passes on the branch, not
  `scoot-reviewer`):
  - **Upward.** The first version compared a count against the ledger. A
    client with fds parked on its own buffer's file (in the harness, 8
    in-process duplicates; in a session, wayland-backend's received-fd queue)
    made the first import learn 4 copies instead of 1, and 4 would have been
    charged to every client for the rest of the session. The difference
    cancels them. `fds_a_client_parks_on_its_buffer_do_not_skew_the_renderer_probe`
    failed on the counting version (`Some(4)` against `Some(1)`,
    `~/evidence/bfl/probe-skew-failfirst-dbec752-plus-test.txt`) and passes.
  - **Downward.** A `wl_shm` pool can be backed by a dma-buf fd, and a
    dropped pool's fd closes on Smithay's own drop thread, concurrently
    with the two counts, so a client could push the difference down toward
    0: the reading that would let the table fill. So an import is not
    measured while any pool record names the buffer's file, and a count
    that fell is not believed. Only a clean reading is kept; an import that
    could not be measured cleanly is charged one copy per plane and the next
    is measured. The race is reasoned from the drop thread's code, not
    reproduced. The guard is pinned by
    `an_import_a_pool_could_disturb_is_not_learned_from`, which fails with
    the guard removed (`Some(1)` against `None`,
    `~/evidence/bfl/pool-guard-failfirst-guard-removed.txt`), and by
    `a_count_that_fell_during_the_import_teaches_nothing`.

  Learned, not assumed: a hardware driver that imports into a GEM handle is
  expected to measure 0 and be charged nothing, which keeps heavy
  legitimate clients on multi-monitor hardware from being charged for
  copies that do not exist. That expectation is reasoned from Mesa's
  source, not measured; `Asahi.md` Test 10 is the check.
- **A cache drain on `wl_surface` destruction** too, not only `wl_buffer`
  destruction: a surface can hold the last reference to a buffer whose
  `wl_buffer` is gone, and the renderer's copies (and pixman's mappings)
  were then kept until some later frame. Measured before adding it: 50
  `YU12` surfaces destroyed with nothing redrawing left 150 copies open
  until the next render; they did not pile up past one round, since any
  later `wl_buffer` destruction drains them.
- **Mappings.** Smithay's `InnerPool` declares its mapping before its fd, so
  it unmaps before it closes: an open pool fd bounds its mapping. The ledger
  therefore also bounds the shm mappings a client keeps, destroyed-but-
  committed ones included, at 512 of up to 512 MiB each. Before, only the
  buffers that were still live objects were bounded.
- **Tests that relied on the old shape.** The bypass-loop test now pins the
  fd bound tripping at the 513th `create_pool` (on `wl_shm`) rather than the
  buffer cap at the 513th `create_buffer`; the buffer-cap tests and the icon
  flood fill the 512-buffer budget from shared pools, so they still reach
  the cap they are about.

## What is still not counted

- **wayland-backend's queues**, below scoot. Received fds parked with
  fd-less requests ([its own ticket](../core/wayland-backend-fd-queue.md),
  unbounded). And, reasoned from wayland-backend 0.3.17's source here and
  not measured: an event carrying an fd (a keymap, a format table, a
  selection `send`) is queued for a client with a *duplicate* of the fd
  until flushed, and a client that stops reading keeps those until its
  4096-byte outgoing buffer is full, when it is disconnected. The smallest
  such events are 12-16 bytes, so a few hundred fds at most, and only after
  the client has filled its socket's kernel buffer.
- **A renderer copy between its plane closing and the next drain.** The
  record goes when the plane closes, and the copy goes at the next drain.
  The drain runs at the loop's next idle after any `wl_buffer` or
  `wl_surface` destruction. The one case with no drain scheduled is a
  commit that replaces a buffer whose `wl_buffer` was already destroyed,
  on a surface nothing redraws; its copies wait for the next drain or
  frame. Bounded by one round of the client's planes (at most 256 copies).
  Review saw all 240 copies of 80 released buffers close within 500 ms on
  both GLES tiers.
- **An output added after a plane was admitted** (before or after its
  import) costs a copy that plane was not charged for.
- **A driver keeping more than one copy per plane**, which none measured
  does: planes admitted before the session's first clean measurement were
  charged one copy per backend and are never raised.

## The interface for the wayland-backend fix

A fix there can meet this in two ways. A transport-level bound (disconnect
past N queued fds) needs nothing from the ledger: it is one more term in the
reserve arithmetic in `fd_pressure.rs`. The room it has depends on which
figure it is added to. In the steady state, one connection at every scoot
bound is 577 fds, 620 with the GPU tier's idle baseline, against the 896
line: 276 to spare. In the software-GLES drain window (copies of planes a
client just released, until the next cache drain) up to 256 more can be
held for a moment, 876 in all, leaving 20. Review measured that window
closing within 500 ms, so a queue bound meant to hold even inside it has
to be small; one sized against the steady state has 276. A per-client count scoot can read would become one more
term in the ledger's per-client total: `ClientFds::admit` reads
`held_by(client)` for both the 512 bound and the pressure grace, and adding
a queued-fd count there (or recording queued fds as another `Kind`) makes
the existing refusals cover it. The syncobj knock-on that ticket names (a
parked syncobj fd reading as another client's dead timeline) is untouched:
identity checks cannot tell syncobjs apart.

## Evidence

Dev VM (kernel 6.18, virtio-gpu, `RLIMIT_NOFILE` 1024/524288). Before is
`1f2fe5c` (`main`), built from a `git archive` into its own target dir
(`CARGO_BUILD_JOBS=1 cargo build --release -p scoot -p scootctl --features
scoot/gpu-scanout`; binary sha256 `e82acc7f…`). After is the PR's code at
`d6d3fe4`, which charges renderer copies at admission (the review fix),
built by the same command in the shared target (sha256 `6f2eac38…`). Every
`--tty --renderer gles` row, the review's overshoot shape on both GLES tiers,
the GPU-tier legit run and the headless 900-surface row were re-run on it
(`runs/*-after-d6d3fe4.txt`). The 500-surface headless row and the `--tty`
shm row are from `82b6590` (`0b180fbb…`); neither imports a dma-buf, the
only path changed since. Earlier runs at `42914bb`, `dbec752` and `0111f57`
gave the same numbers for every row they share. Raw output:
`~/evidence/bfl/runs/*.txt`; probe source `~/evidence/bfl/probe/src/main.rs`
(and the reviewer's `overshoot` mode, `~/review239/probe`, run read-only
through `~/evidence/bfl/over.sh`); runners
`~/evidence/bfl/{run,legit,bench,scanout,over}.sh`. Gates:
`~/evidence/bfl/gate-d6d3fe4.out` (earlier: `gate-0111f57.out`,
`gate-82b6590.out`).

**Fail-first** against `1f2fe5c` (`~/evidence/bfl/failfirst-main-1f2fe5c.out`,
scratch source `~/evidence/bfl/scratch/failfirst_bfl.rs`), 3 of 3 failed:

- shm loop: `outcome=client survived surfaces_attempted=576 tagged_server_fds=576 tagged_mappings=576 wl_buffers_counted=0 pools_counted=0`.
- GLES `YU12` loop: `outcome=client survived buffers_attempted=200 planes_each=3 server_dmabuf_fds=1200 wl_buffers_counted=0` (600 planes plus 600 renderer copies).
- reserve arithmetic on main's constants: `one connection counted=865 + gpu idle baseline 43 = 908 against line 896`.

**Live** (release binaries, `~/evidence/bfl/runs/`):

| Shape | Tier | Before (`1f2fe5c`) | After (`d6d3fe4` unless noted above) |
|---|---|---|---|
| 900 surfaces, each keeping a destroyed buffer and pool | `--headless` | 919 fds; `wayland-info` 0 globals; `scootctl` refused (pressure); honest client reset; attacker connected after 12 s | refused at 512 surfaces (`invalid_stride` on `wl_shm`, "512 file descriptors ... the maximum is 512"); 18 fds after; `wayland-info` 38 globals; `scootctl` served; honest client 147 commits, frame callbacks p95 21.5 ms |
| 500 surfaces (under the bound) | `--headless` | 519 fds, all served | 519 fds, all served (same) |
| 200 `YU12` surfaces, `wl_buffer`s destroyed | `--tty --renderer gles` | table full: 1024 fds (984 `/dmabuf:`); `wayland-info` 0 globals; `scootctl` and the honest client reset; attacker stuck | refused at 85 buffers (the 3rd `add` of the 86th; 6 fds each with llvmpipe's copies); 46 fds after; newcomer 39 globals; `scootctl` served; honest client 134 commits |
| 85 `YU12` surfaces (at the bound) | `--tty --renderer gles` | — | 557 fds total (517 `/dmabuf:`), newcomers, `scootctl` and the honest client served: one client at the bound stays 339 under the 896 line |
| 900 shm surfaces | `--tty --renderer gles` | — | refused at 512, newcomers and `scootctl` served |
| Review's overshoot: 80 committed `YU12`, 30 planes pending, all imported | `--tty --renderer gles` | at `0111f57` (import-time charging): 587 fds (547 `/dmabuf:`), client connected, and again after a refill | refused during the pending adds (`no_memory`, "512 file descriptors"); 527 fds at the refusal |
| the same | `--headless --renderer gles` | — | refused during the pending adds; 506 fds at the refusal |

The session log records `dmabuf: learned how many fds the renderer keeps of
each imported plane copies_per_plane_per_output=1` on the `--tty` GLES tier.

**Direct scanout keeps no further fd per buffer** (`runs/tty-scanout8-{before,after-0111f57}.txt`,
runner `scanout.sh`): a fullscreen client alternating 8 card0 dumb dma-bufs
on `--tty --renderer gles` went direct (`zero_copy` 305 of 305 presented).
Mid-run scoot held 23 `/dmabuf:` fds. That is the 7 of its own every
GPU-tier run holds once it renders, plus 8 x 2 (each plane and llvmpipe's
copy of it). The scanout import through GBM/KMS keeps none beyond those,
the same 23 before and after, so the bound's arithmetic holds on the
direct-scanout path too.

**Legitimate clients**, 5 s each, before and after, both tiers
(`runs/legit-{headless,tty}-{before,after}.txt`, `runs/legit-headless-after-82b6590.txt`, `runs/legit-tty-after-d6d3fe4.txt`): `foot`, `zenity` (GTK 4),
`es2gears_wayland`, `eglgears_wayland`, `vkcube`, `mpv --vo=gpu` all alive
with one window and identical scoot fd counts before and after; an
explicit-sync client (card0 dumb buffers) 177-179 commits, 0 reuse timeouts on
the GPU tier. No refusal or protocol error logged but one xdg-activation
token refusal, identical before and after. `mpv --vo=dmabuf-wayland`
(the one multi-plane video client available) cannot upload frames to a DRM
buffer on this VM, before and after alike (`hwupload` fails), so no real
multi-plane client ran; multi-plane buffers were exercised by the probe and
the harness.

**Cost.** End-to-end (`~/evidence/bfl/bench-churn-*-400k.txt`, headless,
scoot CPU jiffies at CLK_TCK 100, 3 alternating rounds, `1f2fe5c` against
`42914bb`; neither the `add` nor the `create_pool` path changed after
`42914bb`, and neither shape imports a buffer): 400 k params x 4
`add`s + destroy went from 85/85/84 to 136/147/136, about 320 ns more per
`add` (the `fstat` for the identity, and the ledger's map work); 400 k
`create_pool` + destroy from 258/252/260 to 260/269/261, within noise of
the `mmap` probe that path already pays. A client pays this once per buffer
it allocates, not per frame. The in-tree microbenches
(`client_fds::tests::{arrival_cost,sweep_cost}`, `--ignored`) were run in a
debug build (`~/evidence/bfl/bench-micro-debug.txt`): the release test
binary (fat LTO, one codegen unit) ran the dev VM out of memory mid-link and
was stopped, so these are upper bounds dominated by syscalls. An arrival
with its identity `fstat` 1.90 us, without (a timeline) 1.66 us, so the
`fstat` is about 240 ns, matching the release end-to-end delta. A sweep
checks a pool or plane record in 259 ns (one `fstat`), a timeline in
1.08 us (`fcntl` + `readlink`): a full 512-fd sweep is about 130 us, and
runs at most once per 16 arrivals, only for a client at a bound or past the
pressure grace. The renderer probe runs once per session.
