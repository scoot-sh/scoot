---
title: "No cap on Wayland connection count (per-connection bounds multiply) — RESOLVED (accept-loop kill fixed; count cap closed as accepted tradeoff)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No cap on Wayland connection count (per-connection bounds multiply) — RESOLVED (accept-loop kill fixed; count cap closed as accepted tradeoff).

## The entry as filed

Every per-client bound is per *connection* (capture frames 16, bind budget
8, live pools 128, live buffers 512) while Wayland connections themselves
are unbounded, so N connections hold N times any allowance: ~2 connections
at both shm caps hold ~1280 fds past the 1024-fd table, denying pools (and
sockets, and mmaps) to every client. Fix direction named two options -- a
connection cap in the IPC shape, or a global pool/fd ceiling -- and flagged
the harsher blast-radius standard: a Wayland cap denies *shells*, so a
miscount kills the taskbar, not an agent socket.

## Verify-first: what the numbers actually say

Re-derived against the current code, not the ticket prose:

**1. Every per-connection bound is real (each test-pinned).** Live
`wl_buffer`s 512 (`wl_buffers.rs:162`; eleven `dispatch/tests.rs` buffer
tests: bypass loop, composition, churn, isolation, bad-fd phantom,
disconnect drain, single-pixel ×2, dmabuf ×3). Live `wl_shm_pool`s 128
(`shm_pools.rs:104`; `dispatch/tests.rs` pool floods). Bind budget 8
(`bind_budget.rs:141`; `bind_budget/tests.rs` per-global flood/isolation/
drain/floor tests). Screencopy frames 16 per client (`screencopy.rs:253`;
`screencopy/tests.rs` flood/innocent-client tests). Activation tokens 64 --
the one bound that is already global (`activation.rs:156`;
`activation/tests/spawn.rs`; holds no fds). Worst-case server-side fds held
by one connection inside every bound: 512 buffer fds (each surviving shm
buffer retains its `Arc<Pool>`'s `OwnedFd`, one fd minimum -- `wl_buffers.rs`
module doc) + 128 live-pool fds + 1 socket ≈ 641, plus ~13 baseline fds for
the compositor itself (measured `/proc/PID/fd` on a dev-VM `--headless`,
2026-09-18; the IPC record's 11 predates the IPC spare fd).

**2. The multiplication breaks at N=2, with nothing tripped.** Two
connections at the buffer cap alone hold 2 × 513 + 13 ≈ 1039 fds against the
1024 soft `RLIMIT_NOFILE` (measured `ulimit -n` / `prlimit`, dev VM,
2026-09-18) -- past the table without either connection ever seeing a
refusal or a kill, since each stays inside every per-connection bound. No
count cap a real session fits through stops this: bar, panels, launcher,
apps and dialogs need dozens of connections, and any cap ≥ 2 still admits
the killing pair. A cap of 1 is absurd on its face.

**3. Smithay does not survive the exhaustion -- the ticket undersold the
bug.** `ListeningSocket::accept` folds only `WouldBlock` to `Ok(None)` and
returns `Err` on `EMFILE` (wayland-server 0.31.14 `socket.rs:144-150`);
Smithay's `ListeningSocketSource` drains with `while let Some(client) =
socket.accept()?` (`socket.rs:114-121` at the pinned rev `0ff0098`), so the
error propagates out of `process_events`, through calloop's
`dispatch_events` (`loop_logic.rs:527`, `?`) and `EventLoop::run`
(`loop_logic.rs:657+`, `?`), out of `compositor::run` (`mod.rs:192`, `?`),
and `main` prints it and exits `FAILURE`. A flood does not take the socket
down, the way the ticket's question assumed. It takes the whole compositor
down -- every client's unsaved state with it. Proven live, not just traced
(see Verification): a pre-fix binary with its table pinned full dies on the
next backlog entry with `flexwm: other error during loop operation: Too
many open files (os error 24)`.

## Resolution (2026-09-18, this PR)

Two halves, matching what the numbers demand rather than what the ticket
sketched:

**The kill is fixed (code).** `compositor/wayland_accept.rs` replaces
Smithay's `ListeningSocketSource` at the one call site (`state.rs`'s
`listen`), with the same bind (`wayland-1`..`wayland-32`, `wayland-0`
skipped for Smithay's stated reason) and the same insert callback
(`insert_client` failure still warns and continues -- that path already
survived; only the pre-callback `accept` killed). The difference is all in
the drain, which maps every error to a `PostAction` the way `ipc/accept.rs`
does and never propagates: `EMFILE`/`ENFILE` sheds one pending connection
per turn through a shared-`Spare` (one fd held for the session; the
spare/shed/classify primitives are shared with the IPC loop under the same
fork-child allocation discipline), a dead listener deregisters, anything
else logs loudly and stays registered. A shed Wayland client sees an
immediate EOF -- the ticket's own point stands: there is no protocol
channel for refusing a Wayland connection gracefully, and no fd to serve
even an out-of-band reason with. Same wire shape as the IPC shed. No
benchmark: the success path is the same accept loop behind one function
call, per-connection rather than per-frame, and steady state costs one
held fd (baseline 13 → 14, measured) -- the PR #66 precedent for skipping
applies exactly.

**The count cap is closed as accepted tradeoff (no code).** A global
connection count cannot fix the arithmetic (any usable count admits the
pair that fills the table) and pays the ticket's own stated blast radius
(a miscount denies shells at startup, killing the taskbar). What the fix
above guarantees is the IPC sibling's guarantee: the compositor survives
either way; under a real flood, greedy connections shed with EOF and
innocent clients' pool/buffer creations fail until pressure lifts. Revisit
if a real workload ever wedges innocents this way -- the same condition as
`connection-cap-denies-the-same-user-done.md`, which this mirrors.

## Tests

Five new tests in `wayland_accept/tests.rs`, mirroring `ipc/accept/tests.rs`
(including its forked-child exhaustion choreography and the serial lock that
keeps this module's own fd churn out of the count-to-fork window -- learned
the hard way: the first green run left a hung child behind when a sibling
test's fds undercounted the table by two; the child now reads non-blocking
so a miscalibration fails fast instead of hanging, and a `serial()` guard
that recovers from poisoning so one failure does not cascade):

- idle listener drains clean, taking nothing;
- two pending connections are served, backlog `Ok(None)` behind them;
- a shed that finds no backlog re-arms the spare;
- the exhausted child: `accept` fails `EMFILE` with the backlog pending
  (pins the mechanism Smithay propagated), then `drain` returns `Continue`
  having served nothing, both shed clients see EOF with no bytes before it;
- post-recovery: a fresh connection is served, not shed.

Fail-first, each toggled Mac-side and restored: the shed arm returning
`Remove` fails exactly the exhaustion test (child reports deregistration);
`re_arm` as a no-op fails exactly the re-arm test (`is_armed` false).
Neither poisons its siblings (checked: 3 pass / 1 fails in the neutered
run). The behavioral kill itself is proven live pre/post fix (below), which
no suite test can do -- driving Smithay's source under exhaustion needs a
registered token, and registering allocates, which the forked child must
not do.

## Verification

Live kill pair, dev VM 2026-09-18 (both `--headless`, debug, `nc -U` as the
raw backlog entry):

- **Pre-fix** (`/tmp/prefix-target/debug/flexwm` built from pristine
  `origin/main` 618b370): 13 baseline fds, soft pinned to 13
  (`prlimit --nofile=13:524288`), one `nc -U wayland-1` → process dead
  within 3s; log's last line `flexwm: other error during loop operation:
  Too many open files (os error 24)`; client got nothing.
- **Post-fix** (`/var/cargo-target/debug/flexwm` built from this branch):
  14 baseline fds (the spare), soft pinned to 14, one `nc -U` → process
  alive, one `WARN ... wayland_accept: out of file descriptors; shed a
  pending wayland connection ...` in the log, client saw EOF (empty
  output). Soft restored to 1024 → a fresh client stays connected, `flexwm
  msg version` answers normally, exactly one shed in the log (no repeat,
  no spin).

Standard set, dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`), against
this branch with the docs and smoke-script fix included (no Rust change
after the runs below -- `git diff` from the runs to the commit is docs and
`scripts/smoke-test.sh` only):

```
cargo test -p flexwm            TEST EXIT=0  874 passed; 0 failed; 1 ignored
                                         (+ 3 passed in the integration binary)
cargo nextest run --workspace   NEXTEST EXIT=0  979 run: 979 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0 (also Mac-side; formatted Mac-side)
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0  (17 `ok:` lines)
```

`cargo test` went 870 → 874: exactly the 4 new `wayland_accept` tests, plus
the pre-existing suites untouched. The smoke run above is the second one:
the first (SMOKE EXIT=1 after 9 oks, every compositor behavior check
passing) tripped a latent script bug the new `wayland-info` on the dev VM
exposed -- the dmabuf check's socket-name grep took an unanchored `head -1`
across the whole log, which always grabs Smithay's earlier
`smithay::wayland::output ... "headless"` line instead of the "flexwm is
up" line, so the name extraction came up empty on any machine with
`wayland-info` installed. Untouched by this PR's compositor diff (the log
format and startup order are byte-identical on `main`), fixed as a
one-line anchor to the `flexwm is up` line in the same run's log
(fail-first pair: old pipeline yields empty, new yields `wayland-1`), then
the full script went green including `ok: zwp_linux_dmabuf_v1 is
advertised`. No compositor code changed for it, per the PR #43 precedent.

No README change: log lines are not user-facing (the PR #66 precedent), and
no config, flag, binding, or IPC surface moved.

## Adjacent, named rather than fixed here

- [A global fd/buffer ceiling across
  connections](../security/wayland-global-fd-ceiling.md) (new, low): the
  ticket's second option, decided against for now -- a shared ceiling would
  kill an innocent client for another's greed (harsher than the IPC
  refusal-with-reason, and the shape `bind_budget.rs` deliberately refused
  to build). Needs its own refusal-form design, not half of this PR.
- `ipc/accept/tests.rs`'s exhaustion child carries the same theoretical
  undercount-hang this module just fixed (its reads block); observed in the
  sibling, fixed here, untouched there -- same suite, same binary, worth
  knowing if it ever hangs.

## VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it. No `--tty` seat was claimed (every live run here is
`--headless` plus raw socket connects). Two footguns logged for the next
session: `cargo test | tail` hides a 10-minute cold compile behind a
silent timeout (run detached, poll a log file), and `pkill -9 -f <pattern>`
matches the invoking `bash -c` command line itself when the pattern
appears in it -- kill by pid file, never by self-matching pattern.
