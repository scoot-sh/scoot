---
title: "Nothing bounds how many manager/list objects one client may bind — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Nothing bounds how many manager/list objects one client may bind — RESOLVED.

## The entry as filed

Nothing bounded how many `ext_workspace_manager_v1` objects one client
could bind. Each cost a registry entry and a handle per workspace, and
every workspace change walked them all. Filed as per-client accounting, not
a one-off limit.

Update 2026-09-16: `ext_foreign_toplevel_list_v1` had exactly the same shape
and was deliberately left to this entry rather than capped on its own — each
bind costs one handle object per *window*, and every window change walks
every bound list. Sized against the more dangerous multiplier: `ext-workspace`'s
per-bind cost is the workspace *count* (user-driven only, a client cannot
inflate it), foreign-toplevel's is the window *count* (one client can create
windows directly and without limit — the 200-in-a-burst case).

Update 2026-09-16 (PR #49): `zwlr_output_manager_v1` is the third of this
shape. Per-bind cost one head plus one mode per known mode — bounded (one
output, modes grow at most rarely) — the least dangerous of the three.

Update 2026-09-16 (PR #50): `zwlr_foreign_toplevel_manager_v1` is the fourth,
sharing the *worst* multiplier (one handle per window). Two consequences for
sizing: the worst case is `(ext binds + wlr binds) x self-created windows`,
so the budget must be shared across globals or a client spends it twice; and
its worst walk is `wl_output` binding (`binds x windows` per bind by *any*
client), cheap per unit (~one id comparison, sends nothing to skipped
clients) but the shape belongs here. (`refresh_wlr_activation` is *not* the
expensive one — its handle loop sits inside the changed-bit branch.)

## Resolution (2026-09-17, this PR)

One shared per-client budget, not four one-off limits: `BindBudget`
(`compositor/bind_budget.rs`), **8 binds per Wayland client across all four
globals**, claimed in each global's `bind` before anything is announced and
released idempotently on its `stop` and in its `destroyed` hook.

### The number, with reasoning

- **Floor (measured, not guessed).** Stock quickshell 0.3.1 (the dev VM's nix
  store build, the one DMS and Noctalia run on) was driven live against the
  post-change binary with `WAYLAND_DEBUG=1`: on its single connection it binds
  exactly **one** of the four globals (`zwlr_foreign_toplevel_manager_v1`,
  version 3) and none of the other three. quickshell 0.3.1 carries no
  ext-workspace or output-management QML singleton (verified in its
  `qmldir`/`qmltypes`: workspaces exist only via the Hyprland/i3 IPC
  modules), and ordinary toolkit clients bind none of the four. So
  steady-state legitimate use is at most one bind per global -- four total --
  and 8 is twice that.
- **What 8 bounds.** Worst case per abusive client: all 8 binds on the two
  window-multiplier globals, i.e. `8 x windows` handle objects per global,
  each carrying its 4-5 announcement events. Against the measured 200-window
  burst that is ~3,200 small objects (a server object of a few hundred bytes
  plus queued events of ~100 bytes each plus client-controlled title/app-id
  bytes) -- low single-digit megabytes -- and a `wl_output` bind then walks
  them at ~30ns per `same_client_as` (PR #68's measured 30-34ns), i.e. ~0.1ms
  at that window count. A tighter number would bound a few more small objects
  per abuser; the failure mode of a false positive is a shell permanently
  missing its taskbar or workspace list, so the margin errs generous.
- **Why not lower/higher.** The dead-but-unpruned analysis below leaves one
  transient (a disconnect racing a bind in the same batch briefly counts both)
  that doubles a count momentarily -- the budget must exceed 2x steady-state
  legit rather than merely exceed it. Higher would grow the worst case
  linearly for no legitimate consumer: no shell binds more than one of any
  global per connection.

### Refusal form per global (from the protocol XMLs, not assumed)

Every global of this shape already has a protocol-designed teardown, so the
refusal uses it -- never a protocol error, which would kill a legitimate
shell on a miscount, and per-client by construction (only the overflowing
client is told):

| Global | Refusal (sent on the fresh object, loop-idle; see below) |
| --- | --- |
| `ext_workspace_manager_v1` | `done` + `finished` (destructor) |
| `ext_foreign_toplevel_list_v1` | `finished` (plain event: "the client should destroy the object"; no more `toplevel` after it) |
| `zwlr_foreign_toplevel_manager_v1` | `finished` (destructor) |
| `zwlr_output_manager_v1` | `done` (current serial, informational) + `finished` (destructor) |

Refused binds are never counted and never registered, so no later refresh or
window walk touches them. There is no refused-with-reason channel (Wayland
has none for `finished`); the reason lives in the compositor log at `debug`,
matching the IPC cap's refused-with-reason as closely as the protocol allows.

### The refusal is deferred past the bind epilogue -- found by testing

The first version sent `finished` inline in `bind` and died: wayland-backend's
bind epilogue unconditionally assigns user data to the just-bound object
(`rs/server_impl/common_poll.rs` `Bind` arm:
`client.map.with(object.id, ...).unwrap()`), and a destructor `finished` sent
inside `bind` destroys the object first, so the `unwrap` panics and takes
every client's session with it. The existing mid-announce give-ups never hit
this only because they run when the client is already gone (the epilogue
skips a dead client). So a refused bind is pushed onto `BindBudget::refused`
and its `finished` goes out from a loop-idle callback (`RefusedBind`), which
runs once the loop goes idle -- normally the same pass -- holding no claim
and no handles in the meantime. A client that destroys the refused object
first is harmless (the send is swallowed as `InvalidId`; the release finds
nothing). Fire-and-forget `insert_idle` is safe: calloop documents that
dropping the `Idle` handle does *not* cancel the callback.

### The fourth global is re-homed, because Smithay's cannot be intercepted

`ForeignToplevelListState` keeps its bound lists and toplevels in private
fields with no accessor, its global data is unconstructible, and its bind
announces unconditionally -- no place to count against the shared budget and
no place to refuse. So `foreign_toplevel.rs` now owns the protocol directly,
mirroring `foreign_toplevel_management.rs` (hand-rolled from the start),
minus the control half this protocol does not have. Wire behavior is
unchanged by construction and by suite: the pre-existing
`foreign_toplevel/tests.rs` (15 tests incl. bind-before/after-window, churn,
stop/destroy teardown, lock liveness) passes unmodified except one helper
reading the renamed map -- plus one ordering fix the suite caught (see
bug-bash). `shell.rs`'s three call sites keep their signatures.

### Client identity and the dead-but-unpruned transient

Keyed by bind-time `ClientId`, never pid (unstable: zero in an invisible PID
namespace, reused after exit). Each counted bind stores its `ObjectId`, and
release removes exactly that generation: the backend mints a fresh serial per
created object and folds it into `ObjectId` equality (0.3.17
`rs/server_impl/{mod.rs,client.rs}`), and `ClientId` slots likewise carry a
fresh serial per connection (`client.rs` `create_client`) -- so id reuse can
neither release nor collide with a new bind. The map drops empty entries, so
the accounting itself cannot leak.

`stop` releases synchronously (a stop-and-rebind in one batch must see the
freed slot); `destroyed` releases for bare destroys and every disconnect.
Both are idempotent by exact-id removal. An earlier draft claimed a bare
destroy briefly double-counted; re-verified against `common_poll.rs`, a
destructor *request* runs its `destroyed` hook inline in the same dispatch,
so destroy-and-rebind already sees the freed slot (pinned by test). The only
transient left is a disconnect racing another bind in the same epoll batch,
whose drops wait for the post-batch closure -- fail-safe (one extra refusal,
never a leak), self-correcting at cleanup, and not reachable from the test
harness on purpose (disconnecting settles first).

### The `wl_output`-bind walk: the bind cap suffices, no walk bound added

The accounting bounds binds, not walks -- the walk shape stays. No separate
bound because: the multiplicand is now capped (<=8 binds per client; windows
bounded only by mapped surfaces, which the screencopy won't-fix precedent
already accepts as unbounded); the unit is a ~30ns lock-free compare; and
each provocation costs the provoker a full `wl_output` bind round-trip plus a
live object of their own (self-limiting). Adjacent, named, not fixed here:
`wl_output` binds themselves are unbounded per client (same family as shm
pools), and per-handle title/app-id bytes are client-controlled strings with
no length cap -- the object count is bounded, the payload per object is not.

### Seam for shm pools and screencopy sessions (documented, not built)

`bind_budget.rs`'s module doc names both interception points: shm-pool totals
claim at `create_pool`/`resize` (the blanket `request` sees the `Client`) and
release at pool destroy; screencopy sessions claim on the manager request that
creates one (likewise visible with its `Client` at the blanket seam even
though Smithay's session handler exposes no identity -- the same trick the
frames guard uses) and release on session destroy. Keyed by `ClientId` with
idempotent release so neither half needs a new identity scheme. Neither half
is implemented here, per the ticket's scope (seam only).

## Tests

Ten new real-client tests (`bind_budget/tests.rs`), each driving the wire plus
the compositor-side count:

- one refusal test per global (9th bind finished with the protocol-correct
  prefix events; client survives; a stop frees the slot and the rebind
  announces);
- the budget is shared (2x each kind fills it; one more of every kind refused
  in bind order);
- per-client isolation (greedy client's 9th refused; second client binds all
  four kinds cleanly);
- disconnect drains to zero and a new client refills;
- the legitimate floor (one of every kind announces, count 4);
- stop-and-rebind in one batch succeeds at the cap;
- destroy-and-rebind in one batch succeeds at the cap (inline `destroyed`).

Fail-first, each toggled Mac-side and restored (no `git stash` from the VM
side per the 9p index-lock gotcha): all four `refuse_bind` call sites forced
false (each global's ninth test fails), the ext-ws `destroyed` release
removed (disconnect-drain fails), the ext-ws `stop` release removed
(same-batch rebind fails). Six toggles, six failures, all restored --
verified by grep plus the green runs below.

## Verification

All captured on the dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`), against
commit `04fa280` (the code + tests + README; this record and the ROADMAP
entry followed as a second commit with no executable change). The one
README-only edit after the test runs touches no code; the benchmark
storm/pair ran before the publish-ordering fix with the bind path identical
(the storm never exercises publish -- binds only -- and is noted as such).

```
cargo test -p flexwm            TEST EXIT=0  753 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  852 run: 852 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0 (formatted Mac-side; VM cannot write through 9p)
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0 (15 `ok:` lines)
```

Baseline before the change was 743 passed (the +10 is the new suite).

### Benchmark: bind-storm before/after, plus the mechanism's own cost

Temporary harness tests (removed before commit), dev VM debug build,
`--test-threads=1`. Storm: 50 bare `xdg_toplevel` windows open, then 64
sequential `zwlr_foreign_toplevel_manager_v1` binds:

```
after  (budget enforced):  64 binds in 1.215s, 8 managers registered
before (refuse neutered):  64 binds in 1.342s, 64 managers registered
```

i.e. the storm stops at 8 (8x50 = 400 handles announced) instead of
registering all 64 (64x50 = 3,200 handles plus ~16k events). Totals are
socket-round-trip-dominated either way, which is the point: the mechanism
adds no visible per-bind cost end to end.

Mechanism micro (200k claim+release pairs over live objects, three runs):

```
3.339us / 3.932us / 4.001us per pair (debug build)
```

~4us of HashMap/HashSet work per bind plus the same per destroy, against
~19ms socket round-trips per bind in the storm above -- noise, and off every
hot path (binds happen at client startup, never per frame/per event), so no
optimization was attempted. Release builds hash cheaper; not re-measured
since the debug figure already settles it.

The `wl_output`-bind walk was not re-timed end to end: at the capped
multiplicand it is `binds x windows` id comparisons at PR #68's measured
30-34ns each (~0.1ms at the 200-window burst), provoked only by a bind that
costs the provoker a round trip -- bounded by analysis, not by a new number.

### Live quickshell (the floor and the tolerance halves)

Stock quickshell 0.3.1 from the nix store against `flexwm --headless`
(post-change binary, rebuilt after every code change; the final re-run is
against commit `04fa280`'s content), `WAYLAND_DEBUG=1`:

```
wl_registry#2.bind(8, "zwlr_foreign_toplevel_manager_v1", 3, ...)
```

-- exactly one bind of the four capped globals on its single connection
(single `wl_compositor` bind), zero of the other three: the budget sits at 8x
measured need, 2x the one-per-global theoretical max. With a real `foot`
window open the shell's handle receives `title`/`app_id`/`state`/`done`
batches normally (wire log) -- a well-behaved single-bind client is
unaffected, as designed. Artifacts: `/tmp/qs-wire4.log` (bind counts),
`/tmp/qs-wire3.log` (with-window run) on the dev VM; compositor, `foot` and
quickshell all killed and confirmed gone afterwards.

Not re-run, and named: `--tty` (the guard is backend-agnostic bind-time code;
no `--tty` run was repeated for this change) and multi-window quickshell
re-probe beyond the single `foot` above.

### Bug-bash notes (findings, not just the happy path)

- The bind-epilogue panic above (destructor `finished` inside `bind`) --
  found because the first test run died in `common_poll.rs:322`, not by
  review. Without the deferral the cap itself would be a compositor crasher.
- The publish-ordering delta (`Title,Done` per handle vs Smithay's
  `Title x N, Done x N`) -- caught by the pre-existing
  `two_lists_in_one_client_each_get_their_own_handle`, fixed to the pinned
  order. Client-visible bytes are identical to the delegated implementation.
- A refused bind stopped or destroyed before the idle callback runs sends its
  `finished` twice -- the second lands on a dead object and is swallowed as
  `InvalidId` (the PR #68 mechanism), never reaching the wire. Analyzed, not
  tested; the swallow is backend-guaranteed.
- The refused queue is bounded by live objects per batch and drained every
  pass -- refusing 1,000 binds in one batch holds 1,000 small entries until
  idle, alongside the 1,000 live objects the binds themselves created. No new
  hole.
- Zero/one-window binds, disconnect-with-live-binds, stop-twice, and
  version skew (budget path is version-agnostic) are all covered by the
  suites above; budget-of-zero/one needs no test because the budget is a
  constant 8, far above the measured floor.

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). No `--tty` seat was claimed (every live run here is
`--headless`). Two 9p gotchas hit, worked around per convention: VM-side
writes fail (`cargo fmt -p` and `git stash` → `Permission denied` on 9p
locks), so formatting ran Mac-side (re-checked clean on the VM) and
fail-first toggles were Mac-side edits. One stale `cargo nextest` from an
earlier timed-out (client-side) invocation was found still running and killed
(it was this session's own).
