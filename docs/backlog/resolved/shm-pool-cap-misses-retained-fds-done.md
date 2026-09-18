---
title: "The live-`wl_shm_pool` cap does not bound fds or mappings — RESOLVED (claims corrected; live-`wl_buffer` cap added)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The live-`wl_shm_pool` cap does not bound fds or mappings — RESOLVED (claims corrected; live-`wl_buffer` cap added).

## The entry as filed

PR #75's pool-count cap is bypassable by construction: `create_pool` →
`create_buffer` → `destroy_pool` in a loop keeps one buffer alive per
iteration (retaining the fd+mapping via `Arc<Pool>`) while the live-pool
count returns to zero. The docs (`shm_pools.rs:25`, module header, README,
resolved entry) assert fd protection the code doesn't provide. Two ordered
pieces: correct the claims first, then decide the real fix (per-client
live-`wl_buffer` counting was the candidate, with a verify-not-assume list:
observability for all buffer kinds, sizing from real clients, refusal form,
release semantics incl. failed creations, relationship to the pool cap).

## Resolution (2026-09-17, this PR)

Both pieces landed. Part 1 corrects every place that asserted the pool
count bounds fds/mappings; Part 2 implements the candidate fix as filed --
every verify-not-assume question checked out, with one finding (the dmabuf
`create_immed` init-before-import order) pinned by test rather than assumed.

### Part 1: the corrected claims

- `shm_pools.rs` module header: no longer motivates the cap with
  `RLIMIT_NOFILE` exhaustion. States what 128 bounds (live pool objects +
  the 128 x 512 MiB sparse address-space envelope) vs what it doesn't
  (fds/mappings retained by surviving buffers, bounded by `wl_buffers.rs`),
  names the bypass loop, and points at the ticket.
- `shm_pools.rs` "What 128 bounds": "128 live pool objects", not "128
  fds ... 128 mappings/objects".
- `dispatch.rs` module doc + `MAX_SHM_POOL_BYTES` doc: pool concurrency
  bounds live objects and the envelope; retained fds/mappings are the
  buffer count's quantity.
- `README.md`: the pool sentence now states the object/envelope bound and
  its limit, and documents the new 512-buffer bound beside it.
- This entry's predecessor
  (`shm-pool-count-cap-done.md`): pointer correction only, history
  untouched -- a dated note at the top redirecting the fds/mappings
  conclusion here.
- `bind_budget.rs`'s shm seam paragraph: extended with the buffer-cap
  pointer (same key, same shape, same refusal-form reason for its own
  table).
- `single_pixel_buffer.rs`: the "no bypass of a limit that should apply"
  sentence was written before any buffer limit existed -- single-pixel
  buffers are now counted (see below for why uniformity requires it).

### Part 2: per-client live-`wl_buffer` counting (`wl_buffers.rs`, 512)

`WlBuffers` (`compositor/wl_buffers.rs`): `live_per_client:
HashMap<ClientId, u32>`, claimed in `dispatch.rs`'s blanket `request`
before delegation, released in its `destroyed` hook -- the same
claim-before-delegation / release-in-`destroyed` shape as the pool count,
the frame cap and the bind budget, as its own counter (different quantity,
different refusal sites).

#### The verify-not-assume list, each checked against the pinned sources

- **Fully observable for all buffer kinds.** Three server-side `wl_buffer`
  factories exist at the pinned rev, verified by enumerating every
  `Dispatch2<WlBuffer>` impl and every `data_init.init`/`create_resource`
  site producing a `WlBuffer` in Smithay's `src/wayland/`: shm
  `create_buffer` (`shm/handlers.rs:173`), dmabuf `create_immed`
  (`dmabuf/dispatch.rs`, init before import), single-pixel
  `create_u32_rgba_buffer` (unconditional init, no failure path). All three
  are claimed; all buffer destroys release through the one blanket
  `destroyed` hook. The dmabuf async `create` is deliberately *not*
  claimed: it creates no object synchronously, and this compositor's
  `failed()` answer creates none later (pinned by test -- see below). The
  only other producer, `successful()`'s `create_resource`, is unreachable
  while `dmabuf_imported` never calls it; the module doc names it as the
  hook point if a future renderer does.
- **Failed import creates the buffer.** `create_immed` inits first
  (`data_init.init(buffer_id, ...)`), *then* calls `dmabuf_imported`, and
  this compositor's `notifier.failed()` on an infallible import posts
  `InvalidWlBuffer`, killing the client and leaving the object for
  disconnect cleanup (`dmabuf/mod.rs:862-973`, traced line by line, not
  assumed). The dmabuf test below proves it end to end: had no object been
  created, one phantom unit would remain; the count reads zero.
- **The new buffer's id is not readable pre-delegation** (`New<WlBuffer>`
  seals it -- wayland-server 0.31.14 `src/dispatch.rs:136-146`, only
  `wrap`, private field), so exact-id tracking is unbuildable and the
  count is scalar. Scalar is *safer* here than ids: immune to ABA reuse by
  construction, and the uniformity argument below needs it.
- **Single-pixel counted too, deliberately.** The release hook sees only
  that *a* buffer died, never which kind -- so excluding cheap buffers
  from claims while releasing for all destroys would drift fail-open
  (create 5 shm buffers, create-and-destroy 5 single-pixel ones, hold 5
  retaining buffers against a count of zero). Uniform counting has no such
  drift; every initialised buffer of every kind claims once and releases
  once. The budget cost of counting a shape that holds nothing is
  negligible against a 512 cap (single-pixel use is ones of buffers).
- **Failed creations: handled, not assumed away.** The guard claims
  unconditionally, *including* creations Smithay is about to refuse --
  replicating upstream's parameter validation to avoid it would couple the
  guard to Smithay's handler logic and drift fail-open on a rev bump.
  Sound anyway, by mechanism: Smithay initialises or kills, never neither
  (every error path in all three creation handlers posts before `init`),
  so a phantom unit lands on an already-dead entry only -- at most one per
  killing connection, noise next to the connection itself, never on a live
  budget. Pinned by test (`buffers == 1` on the dead entry, documented as
  the stated residual rather than left to drift).
- **Refusal form, per interface.** Protocol error on the creating object,
  killing only the offender (a silent ignore would leave the uninitialised
  object that panics the compositor): `InvalidStride` on the pool (Smithay's
  own bad-buffer-parameter code there), `InvalidWlBuffer` on the dmabuf
  params (Smithay's own immed-failure code), and a bare 0 on the
  single-pixel manager -- which defines no error enum at all (verified
  against the protocol XML: no `<enum name="error">`). Only a client
  already holding 512 buffers ever sees the 0; the kill is the message.
- **Sizing from measurement.** `foot`: exactly 2 live buffers steady --
  double-buffered, reused across typing *and* resizes (zero destroys in
  the whole session), 2 pools / 2 `create_buffer` / max-concurrent 2
  (`WAYLAND_DEBUG=1` wire log, dev VM, 2026-09-17). quickshell: not
  re-measured in this pass, so reasoned not measured (a panel of a handful
  of layer surfaces, double-buffered each -- under 10; a real measurement
  wants a real shell config -- follow-up work). Video-ish: a few queued frames on top of UI buffers.
  Heaviest reasoned: ~60 (20-window browser, triple-buffered). 512 is ~8x
  that and 256x the measured floor -- death-penalty sizing, same doctrine
  as the frame cap (16 for a legitimate 1). Per-connection fd arithmetic:
  512 buffers + 128 live pools ≈ 640 worst case against a 1024-fd table --
  one connection alone cannot exhaust it; two can, which is
  connection-count territory, and
  [wayland-connection-cap](../resolved/wayland-connection-cap-done.md)'s
  "8 connections" math is updated to say so.
- **Relationship to the pool cap: both stay.** Different quantities (live
  pool objects + address-space envelope vs retained fds/mappings), neither
  subsumes the other. The pool cap is untouched -- no behavior change
  there, only corrected docs.

### What landed

- `wl_buffers.rs`: policy, number, uniformity/phantom/maintenance
  arguments, per-connection scoping -- all stated with sources, same
  module-doc doctrine as `shm_pools.rs`.
- `dispatch.rs`: `reject_excess_buffer` (three `TypeId`-gated factories;
  async `create`, `resize`, `destroy`, `add` all fall through) chained
  after the pool guard; `forget_destroyed_buffer` (`wl_buffer`-gated)
  before the delegate, beside the other forget hooks; `too_many_buffers()`
  refusal message; "Why the sixth guard exists" module-doc section.
- `state.rs` / `mod.rs`: `wl_buffers: WlBuffers` field + init, same
  position and doc shape as `shm_pools`.
- `single_pixel_buffer.rs` + its RGBA test: doc corrected (pool budget
  untouched, buffer budget claimed), test asserts both counts (pools 0,
  buffers 3).
- `wayland-connection-cap.md`: per-connection retained-fd bound is now
  the buffer cap's 512 (plus 128 live pools), i.e. ~2 connections to reach
  1024, not 8 -- with the pointer to this record explaining why the old
  math used the wrong cap.
- `docs/backlog/README.md` index + `ROADMAP.md`: this entry marked
  resolved with the new bound.

### What this does NOT bound (stated, not minimised)

- The byte total (still needs-upstream, unchanged).
- Cross-connection multiplication (connection-count territory, unchanged).
- The ≤1 phantom unit per killing connection (dead entries only; see
  above). No disconnect sweep exists or is needed: phantoms attach only to
  dead clients, and live-client accounting is exact.
- quickshell's exact buffer concurrency (not re-measured -- follow-up,
  reasoned instead, 50x headroom).

## Tests

Eleven new real-client tests in `dispatch/tests.rs` (plus a `BufferClient`
helper holding one connection across create/destroy/disconnect steps, a
`BufferDispatch` binding all three factories, batched floods flushed every
64 iterations so 513 client memfds never sit next to 512 retained server
fds under a 1024-fd table, and a shared `FD_FLOOD_LOCK` -- which the three
pre-existing pool floods also take, since one process shares one fd
table):

- bypass loop (513 `create_pool`/`create_buffer`/`destroy_pool`): the
  513rd `create_buffer` refused with `InvalidStride` on `wl_shm_pool`,
  offender dead, both counts drain to zero, survivor unaffected;
- pool destroyed with 5 live buffers, budget filled the bypass way: 512
  total succeeds (the five still count), 513rd refused;
- double-buffer churn (create-third/destroy-oldest x20) + 8-frame burst:
  ten live max, nothing refused, all released;
- isolation: greedy at exactly 512 (zero live pools) while a second
  client takes its first buffer cleanly;
- bad-stride creation: `InvalidStride` kill, exactly one unit on the dead
  entry (the pinned phantom);
- disconnect with 3 live buffers (+ live pool): both counts drain to zero;
- three single-pixel buffers: served, released, drain zero;
- 513 single-pixel buffers: refused with code 0 on the manager, drain
  zero, survivor unaffected;
- dmabuf `create_immed` past a full shm budget: refused with the guard's
  `InvalidWlBuffer` (7) *before* Smithay's validation -- not the
  `InvalidFormat` (4) the garbage format would otherwise earn, which is
  what proves the shared budget caught it;
- dmabuf async `create` past a full budget: still answered `failed`, client
  left alive (proves `create` claims nothing);
- failed dmabuf import (`create_immed`, valid params): `InvalidWlBuffer`
  kill, count zero afterwards (proves init-before-import: a
  never-initialised object would leave one phantom).

Fail-first, each toggled Mac-side and restored (no `git stash` from the
VM side per the 9p index-lock gotcha):

- shm claim neutered: bypass loop fully accepted (513 live, no refusal) --
  the pre-fix unbounded run, bounded in-test.
- release removed from the hook: disconnect-drain fails.
- single-pixel branch neutered: 513 single-pixel buffers accepted.
- dmabuf branch swapped to `Create`: the `create` test fails (client
  killed) *and* the immed test fails with code 4 instead of 7 -- each
  branch's sensitivity proven in both directions.
- churn/burst, isolation survivors, small-single-pixel and disconnect
  pins pass neutered, which is the expected split (they pin shapes, not
  refusals).

One harness finding, fixed, not worked around: the floods hold 513 client
memfds next to 512 retained server fds -- past `RLIMIT_NOFILE` even solo,
and three floods on four test threads exhaust the shared table for
unrelated tests too (the first green run failed two pre-existing pool
tests with `EMFILE` at *their* memfd). Floods now flush every 64
iterations (client peak ≈ 64; the server already holds its own dup plus
the retained mapping, so closing the client's copy changes nothing the
count observes) and the five fd-heavy tests share one lock.

## Verification

All captured on the dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`),
against commit `0c906e7dd961790d13205eff9568e61887fadd05` (the code +
tests + README; this record and the ROADMAP entry followed as a second
commit with no executable change):

```
cargo test -p flexwm            TEST EXIT=0  870 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  975 run: 975 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0 (formatted Mac-side; VM cannot write through 9p)
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0  (17 `ok:` lines)
```

Baseline arithmetic, not a separate run: 870 here, and the diff adds
exactly the 11 new buffer tests while touching three pre-existing pool
tests (flood-lock acquisition only) and one single-pixel test (an added
assertion) -- no test added, removed or renamed otherwise. The dispatch
suite went 15 -> 26.

### Benchmark: the added per-`create_buffer` cost

Temporary harness test (200k claim+release pairs over one live inserted
client, dev VM debug build, `--test-threads=1`, removed before commit),
three runs + empty baseline:

```
claim+release pair: 1685.8/1602.5/1559.8ns/iter (debug)
empty loop: 10.0ns/iter
```

i.e. roughly 0.8us per `create_buffer` and per buffer destroy -- the same
shape as the pool guard's measured 0.75-0.9us and the frame guard's
0.7-0.9us. Cost shape, stated explicitly: the guard runs on every request
of every interface, but for every interface other than the three
factories both sides of the `TypeId` comparison are compile-time constants
once monomorphized and the body folds away exactly like the six guards
before it (no measurement needed beyond the shared argument); the destroy
hook is one integer compare per destroyed object of any kind. `create_-
buffer` is per-buffer, never per-frame: a 513-flood pays ~0.5ms total,
steady-state clients (2 live buffers) pay nothing measurable. Merged on
bound-not-performance grounds all the same; no optimization attempted.

### Live `foot` (the tolerance half of the refusal-form decision)

The smoke test above spawns a real `foot`, types into it and screenshots
it: a well-behaved 2-buffer client is unaffected, as designed (512 is 256x
its need). Separately (this work, dev VM, pre-change binary):
`WAYLAND_DEBUG=1 foot` on `--headless` -- 2 pools, 2 `create_buffer`, max
concurrent live 2, zero destroys across typing and a tiled-resize
churn (`/tmp/bufmeas-foot.log` on the Mac, since removed with the VM
processes). No separate `--tty` run: the guard is backend-agnostic
dispatch code, and the smoke suite is the backend-agnostic net.
quickshell's buffer concurrency was not re-measured in this pass (a live
quickshell binary exists on the dev VM, but a real measurement wants a
real shell config -- follow-up work), with 50x reasoned headroom in the
sizing above.

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). No `--tty` seat was claimed (every live run here is
`--headless`). The 9p gotchas were worked around per convention
(formatting Mac-side, fail-first toggles as Mac-side edits). Measurement
compositor + `foot` + second `foot` were `pkill`ed and confirmed gone
afterwards (`pgrep` clean).
