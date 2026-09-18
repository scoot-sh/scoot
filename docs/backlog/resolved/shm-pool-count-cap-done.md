---
title: "No upper bound on *total* shm reservation per client — RESOLVED (live-pool count capped; byte total needs upstream)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No upper bound on *total* shm reservation per client — RESOLVED (live-pool count capped; byte total needs upstream).

> Correction pointer (2026-09-17, not a rewrite of the record below): this
> entry's "What the count bounds per connection is fds and mappings"
> conclusion is wrong -- a buffer outlives its pool object, retaining both
> (see [the follow-up](./shm-pool-cap-misses-retained-fds-done.md),
> resolved in
> [shm-pool-cap-misses-retained-fds-done.md](./shm-pool-cap-misses-retained-fds-done.md)
> with a per-client live-`wl_buffer` cap). What the 128-count bounds is
> live pool objects plus the address-space envelope, not fds or mappings.
> Everything else below stands as written.

## The entry as filed

No upper bound on *total* shm reservation per client (LOW/MEDIUM).
Found by `flexwm-reviewer` while confirming item 12(d)'s per-pool cap
actually closed the original finding — it doesn't, fully. `dispatch.rs`'s
`MAX_SHM_POOL_BYTES` (512 MiB) bounds one pool, but nothing bounds how many
pools one client opens: 40 pools each at exactly the cap reserve ~20 GiB
from a single connection, more address space than the pre-fix 8-pool/16.1
GiB finding item 12(d) was written to close, just needing more requests to
get there. Not the separate IPC-connection-cap Backlog entry's territory
(that one is about the control socket's own connection count, unrelated to
wayland client accounting). Fix direction: per-client cumulative tracking
in the same `dispatch.rs` interception point, with its own cap — needs
deciding what identifies "one client" for accounting purposes (the
`ClientId` `dispatch.rs` already has access to) and where to hang the
running total (`ClientState`, most likely, alongside the existing
per-client data already tracked there).

## Resolution (2026-09-17, this PR)

Half landed, half proven unimplementable at the pinned rev. What this
compositor now bounds is per-client live-pool *concurrency*
(`shm_pools.rs`, 128 pools per Wayland client, claimed at `create_pool`
before delegation and released in `dispatch.rs`'s pool destruction hook).
What it still does not bound is the byte total the entry asked for -- and
that half is closed as NEEDS-UPSTREAM, not deferred: the implementation
below records exactly which upstream accessor would unblock it, verified
against the pinned sources rather than assumed.

### Why the byte total cannot be built here

An exact byte total needs each pool's size at two points: claimed at
`create_pool`, released at pool destroy. Neither point can observe both
halves through public API at the pinned rev, and all four walls were
re-verified in source (not from general Smithay knowledge):

- The new pool's id is sealed inside `New<WlShmPool>`: `pub struct New<I>
  { id: I }` exposes no accessor, no `Deref`, no field access
  (wayland-server 0.31.14 `src/dispatch.rs:136-146`). The blanket
  `request` seam therefore sees the *size* without the *id* at create
  time.
- The pool's size is unreadable afterwards: `ShmPoolUserData { inner:
  Arc<Pool> }` has a private field and no `impl` block at all (pinned
  `0ff0098` `src/wayland/shm/mod.rs:470`), `Pool` is unexported, and
  `ShmState` keeps no pool registry (`{ formats, shm: GlobalId }` only).
  The destruction hook therefore sees the *id* (`resource.id()`) without
  the *size*.
- `DataInit` (the only other witness to the creation) has `pub(crate)`
  fields, and no server API enumerates a client's objects (`Client` and
  `DisplayHandle` expose `object_from_protocol_id` / `object_info` by id,
  never a listing -- full `pub fn` surfaces checked on both types).
- Size-keyed workarounds fail too: keying by the passed fd's `(dev, ino)`
  dies at destroy (the fd is locked inside the private user data, and fd
  numbers recycle); matching creates to destroys by order is genuinely
  ambiguous (create A, create B, destroy B); and `fstat`-at-create
  (`size > backing` refuses sparse pools) dies at `resize`, which grows
  the mapping with no fd in the request -- and would refuse `foot`, which
  allocates 512 MiB pools on small backing files (measured below).

Approximations (LIFO size assumptions, count x per-pool-cap as a
"conservative total") all drift fail-open under mixed-size create/destroy
orders, so there is no honest partial byte bound -- only the count below,
which needs no sizes at all.

### What landed instead: a per-client live-pool count

`ShmPools` (`compositor/shm_pools.rs`): `live_per_client:
HashMap<ClientId, u32>`, claimed in `dispatch.rs`'s blanket `request`
before delegation, released in its `destroyed` hook -- which also drains
disconnects (whose cleanup destroys every object) and protocol-error
kills. A refused creation is never counted. The count is exact rather
than approximate: only sizes that *will* create a pool reach the claim
(`1..=MAX_SHM_POOL_BYTES` on an fd that maps -- sizes checked in the
guard itself rather than leaned on chain order, fds probed with the exact
mapping Smithay is about to make; see bug-bash), and every counted pool
pairs with exactly one destruction. The probe closes the deterministic
leak review found on the way (valid size, unmappable fd: Smithay posts
`InvalidFd` without initialising, whose no-op `destroyed` never reaches
the hook -- one dead unit per connection, attacker-paced, no memory
pressure at all); what remains is a cross-thread TOCTOU between the probe
and Smithay's own call, stated with the probe in `dispatch.rs`.

What the count bounds per connection is fds and mappings, not bytes: each
live pool holds at least one compositor fd (`InnerPool` owns an `OwnedFd`)
plus a mapping and a `Pool` object. With the dev VM's soft
`RLIMIT_NOFILE` at 1024, ~1000 pools from one connection exhausts the
compositor's fds for *every* client -- the sharp, cross-client edge of
this entry, reachable today with no cap at all. The byte envelope stays
what the per-pool cap says (128 x 512 MiB sparse); the residual is stated
below, not minimised.

### The number, with reasoning

- **Floor (measured, not guessed).** `WAYLAND_DEBUG=1` wire logs on the
  dev VM, one window each: `foot` holds **2 pools of 512 MiB each** (a
  double-buffered arena -- real ~2 MiB framebuffers placed at offsets
  inside it; both destroyed at exit), KeePassXC (Qt) holds **2 pools
  totalling ~8 MiB**, an idle quickshell holds **none**. No resizes in any
  log.
- **What 128 bounds.** 64x the measured single-window floor, ~3x the
  heaviest reasoned legitimate use (~40 pools: a 20-window browser at ~2
  pools per surface). Per connection: 128 fds (an eighth of a 1024-fd
  `RLIMIT_NOFILE`), 128 mappings/objects, ~128 MiB of page tables worst
  case. The margin errs generous on purpose: tripping this disconnects
  the client, so a miscount must not kill a heavy-but-legitimate session
  -- the same death-penalty sizing the frame cap (16 for a legitimate 1)
  already uses.
- **Per connection, not per machine.** Wayland connections are unbounded:
  N connections hold up to 128N pools. Stated rather than solved --
  still strictly better than unbounded per connection, the same
  per-connection shape as the capture-frame cap and the bind budget, and
  cross-connection abuse is connection-count territory. Filed as its own
  item ([Wayland connection cap](../security/wayland-connection-cap.md))
  rather than left as prose.
- **Adjacent measurement, recorded because it constrains the future byte
  total:** `foot` alone reserves 1 GiB under any byte cap (2 x 512 MiB),
  so a future byte total needs to sit well above 1 GiB for the reference
  terminal alone -- and the per-pool cap sits *exactly* at `foot`'s arena
  size (536870912 == 512 MiB), one byte more and the reference terminal
  dies. Neither is changed here; both are now on record.

### Refusal form (consistent with the per-pool cap)

`InvalidStride` on `wl_shm` -- the same code on the same object the
per-pool cap's own creation refusal posts. One consistent answer for
"this pool cannot be opened", whichever bound said so, killing only the
offending client (a silent ignore would leave the uninitialized object
that panics the compositor -- the argument `dispatch.rs` already makes
for the per-pool cap).

### Shared budget vs own counter (decided, not shoehorned)

Own counter. Pools are counted objects with a kill-the-offender refusal;
binds are teardown-with-`finished` ones; one table across those two
refusal forms (and across counts-vs-bytes units) would be the awkward
shoehorn the `bind_budget.rs` seam warned against. What is shared is the
pattern language, not the table: `ClientId` key, claim before
delegation, idempotent release in `destroyed`, map drops empty entries.
The seam paragraph in `bind_budget.rs` is corrected to point here rather
than left describing a future that landed differently.

### Storage (State-side, not ClientState)

The entry suggested `ClientState`; both capped predecessors
(`frames_per_client`, `BindBudget`) already resolved that to State-side
maps, and for a mechanical reason on top of precedent: `Client::get_data`
hands out only `&Data`, so per-request mutation in `ClientState` would
need a lock, while a State-side map keyed by `ClientId` needs none.

### Resize, zero-size, overflow (edge cases, decided)

- **`resize` neither claims nor frees** -- it grows the pool it names.
  Bytes-via-resize stay inside the same envelope (count x 512 MiB, each
  grow still under the per-pool cap), so there is no resize bypass to
  close and none is claimed. A shrink attempt never reaches accounting:
  upstream kills the client for it (`InvalidSize`), whose cleanup drains
  through the same hook. Pinned by test (130 successive grows hold a
  count of one). (Precision note for the byte-total half: the interceptor
  *can* read the resize size -- it already does for the per-pool cap --
  but that saves nothing, because with create-time id-to-size binding
  impossible there is no per-pool entry a resize could update, and
  destroy-time size is unreadable. Any attribution would be order-based
  and drift fail-open: create 512, create 1, destroy the 1-pool, and a
  FIFO pops 512 -- releasing 512 for 1 freed, understating the total
  repeatably back toward zero while real usage persists. The conclusion
  stands: no honest partial byte bound.)
- **Zero-size (and negative) creations consume nothing.** Upstream refuses
  them with `InvalidStride` and kills the client, creating no pool -- so
  counting one would leak a unit no destruction could release, one dead
  map entry per abusive connection. The guard carves `size <= 0` out
  before the claim. Pinned by test.
- **No integer-overflow path.** The counter is a per-client `u32`, and
  every live pool holds at least one compositor fd, so live pools can
  never approach `u32::MAX` (fd exhaustion at ~2^10 here, hard caps at
  ~2^20 anywhere) -- unconstructible, not merely unlikely. Increment and
  decrement saturate anyway, belt-and-braces. No byte arithmetic exists
  to overflow, which the byte-total design would have had to defend.

## Tests

Eight new real-client tests in `dispatch/tests.rs` (plus a `PoolClient`
helper holding one connection open across create/destroy/disconnect
steps, and a `drive_both` two-connection form so the first connection can
stay up -- holding its pools -- while the second runs):

- 129 pooled creations in one batch: the 129th is refused with
  `InvalidStride` on `wl_shm`, the flooding client dies, the count drains
  to zero and the survivor is unaffected;
- create-128/destroy-128/create-1: the 129th *lifetime* pool succeeds --
  every destroy released its unit;
- greedy client sitting exactly at the cap while a second client creates
  its first pool: both succeed (the anti-shared-table property);
- a per-pool-over-cap creation consumes no budget (count still zero);
- a zero-size creation consumes no budget (count still zero);
- a valid-size creation on an unmappable fd (`/dev/null`, which cannot be
  mapped `SHARED`) is refused with Smithay's own `InvalidFd` and consumes
  no budget (count still zero);
- three pools created then the connection dropped without destroying:
  count drains to zero;
- one pool grown 130 times: count holds at one.

Fail-first, each toggled Mac-side and restored (no `git stash` from the
VM side per the 9p index-lock gotcha):

- STAGE 1 (counting without refusal): the flood test fails with "the
  pool size was accepted"; the other six pass -- the true fail-first run.
- refusal forced false: flood fails.
- release removed from the hook: six fail (headroom, drain, flood,
  oversized, zero-size, resize -- the last three via the survivor's own
  pool never releasing, which is the expected split: they pin the
  release path from the other side).
- claim moved before the size checks: zero-size fails (one leaked unit);
  oversized still passes, correctly -- the per-pool guard short-circuits
  first in chain order, which is what protects it; the guard's own
  re-check is belt-and-braces against reorder.
- the bad-fd test failed before the probe existed (`pools == 1`, the
  deterministic leak, confirmed end to end) and passes after -- the
  probe's own fail-first run. No toggle was needed: the pre-probe tree
  *was* the neutered state.
- isolation, composition and resize pins pass neutered, which is the
  expected split (they pin shapes, not the refusal): isolation fails iff
  the key globalises, composition iff refused-oversize starts counting,
  resize iff resizes start counting.

One harness bug found by the new tests, fixed, not worked around: the
pools read raced disconnect cleanup (the drain loop exits as soon as the
client thread's flag lands, possibly before the survivor's own disconnect
is dispatched -- `pools == 0` then fails intermittently). `drive_both`
now drains up to 50 zero-timeout dispatches after joining before reading
the count; the suite went from rotating failures (~3/6 runs) to 10/10
green. Pre-existing tests are unaffected (they never read the count).

## Verification

All captured on the dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`),
against commit `ae870bf7f5835070280ed2ed38f1137839423a85` (the code + tests + README; this record and the
ROADMAP entry followed as a second commit with no executable change):

```
cargo test -p flexwm            TEST EXIT=0  762 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  861 run: 861 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0 (formatted Mac-side; VM cannot write through 9p)
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0 (15 `ok:` lines)
```

Baseline before the change was 753 passed at PR #74's commit, 754 past
its review-fix commit (which added a test alongside the refused-stop
dedup), and 762 here -- the +8 is exactly the new suite. The dispatch
suite alone went 7 -> 15 and ran 10/10 green after the drain fix.

### Benchmark: the added per-`create_pool` cost

Temporary harness test (200k claim+release pairs over one live inserted
client, dev VM debug build, `--test-threads=1`, removed before commit),
three runs:

```
empty loop:              10-12ns/iter
claim+release pair: 1.50/1.74/1.60us/iter
```

i.e. roughly 0.75-0.9us per `create_pool` and per pool destroy in a debug
build -- the same shape as the capture-frame guard's measured 0.7-0.9us
(PR #70). The fd-mappability probe adds one `mmap`+`munmap` per creation,
measured separately the same way (200k probes of a 4 KiB memfd, three
runs: 2.82/2.80/2.83us per probe, debug) -- noise against requests that
are socket-I/O-dominated and arrive at most in the hundreds per client
lifetime (even a 129-pool flood pays ~0.5ms total, probe included).
Crucially, the probe does not false-refuse sparse-but-mappable pools: the
`creating_a_pool_up_to_the_cap_is_accepted` suite maps 512 MiB on a
one-byte backing through the probe on every run. Every other interface's cost is a `TypeId` comparison both
sides of which are compile-time constants once monomorphized, folded away
exactly like the five guards before it. Merged on bound-not-performance
grounds all the same.

### Live `foot` (the tolerance half of the refusal-form decision)

The smoke test above spawns a real `foot`, types into it and screenshots
it: a well-behaved 2-pool client is unaffected, as designed (128 is 64x
its need). No separate `--tty` run: the guard is backend-agnostic
dispatch code, and the smoke suite is the backend-agnostic net.

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). No `--tty` seat was claimed (every live run here is
`--headless`). The 9p gotchas were worked around per convention
(formatting Mac-side, fail-first toggles as Mac-side edits). One
self-inflicted `pkill -f` matched its own ssh command line during pool
measurement and killed the measurement session mid-command; the
compositor and clients it had started were confirmed gone afterwards and
the measurement re-run cleanly.
