---
title: "The crate, the daemon and the control socket"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# The crate, the daemon and the control socket — RESOLVED

Resolved 2026-09-26. What landed, where it departs from the plan, what
is left for later tickets, and the measurements are in
[Resolution](#resolution) at the end; the original ticket follows
unchanged.

The first real PR, and the one that creates the crate.

- Add `crates/scootbg` (the workspace's `crates/*` glob picks it up),
  MIT, `description` saying "wallpaper daemon for Wayland" (not
  "compositor": it is a client). A short `crates/scootbg/README.md` may
  point at `docs/scootbg/`, which stays the home of its docs.
- Add `crates/scootbg-mem` beside it: the only `unsafe` in scootbg.
  - Two modules: the large-allocation global allocator and the `wl_shm`
    buffer mapping (sealed memfd). Each `unsafe` block carries a
    `// SAFETY:` comment, enforced by clippy's
    `undocumented_unsafe_blocks` lint.
  - Pure Rust (`rustix`, no `libc`), `publish = false`.
  - `scootbg` is `#![forbid(unsafe_code)]`.

  See the
  [design and safety arguments](dependencies-done.md#11-scootbgs-own-unsafe-two-modules-in-one-small-crate).
- One binary with subcommands: `daemon` runs the Wayland client, and every
  other subcommand is a client of its control socket.
- Control socket at `$XDG_RUNTIME_DIR/scootbg-$WAYLAND_DISPLAY.sock`, so
  two sessions (a nested scoot inside another) never share one.
  `WAYLAND_DISPLAY` may be an absolute path, which libwayland allows, so
  derive the name from its final component (or a hash) rather than
  splicing it in. Line-framed
  JSON requests and replies, versioned from day one like `scoot-ipc`.
  Self-contained, not reusing `scoot-ipc`: it would pull no compositor
  types, but it would share only ~25 lines of generic framing and would
  put `crates/scoot-ipc/` on scootbg's CI path
  ([decided](dependencies-done.md#5-serialization-control-socket-and-state-file)).
- Dependencies as decided in
  [`resolved/dependencies-done.md`](dependencies-done.md), each
  with a one-line reason in `Cargo.toml` comments linking there. Weigh the
  release binary with `cargo build --release -p scootbg`, not a
  `--workspace` build (feature unification).
- A second `scootbg daemon` on the same display refuses loudly (socket
  already live) instead of stacking a second set of surfaces; a stale
  socket from a crashed daemon is detected and replaced.
- A Wayland disconnect (compositor exit) ends the daemon cleanly with a
  non-zero status, never a panic.
- Nix: a `scootbg` package output of its own (the `scoot` package stays
  `scoot` only, as with `scootctl`) and a place in the dev shell.
- CI, split by path, extending the `changes` job in
  `.github/workflows/ci.yml` (which today only skips docs-only PRs):

  | Changed | Runs |
  |---|---|
  | `crates/scoot/`, `crates/scoot-core/`, `crates/scoot-ipc/`, `crates/scootctl/`, `scripts/smoke-test.sh`, `vm/compositor-deps.nix` | the scoot jobs |
  | `crates/scootbg/`, `crates/scootbg-mem/` | a scootbg job (fmt, clippy, tests) |
  | `Cargo.toml`, `Cargo.lock`, `flake.*`, `nix/`, `.github/` | everything |
  | anything not listed (`.config/nextest.toml`, `resources/`, `devenv.*`, other `scripts/` and `vm/` files, `LICENSE`) | everything |
  | either side | the scootbg-on-headless-scoot integration test |

  Match on directory prefixes with the trailing slash: `crates/scoot*`
  would also match `crates/scootbg`, and `crates/scootbg/` does not match
  `crates/scootbg-mem/`, which is why both are listed.

  The integration test runs for both because the `apply-config` contract
  couples them: a scoot change can break scootbg's end-to-end run and the
  reverse. Gate jobs with `if:` on the `changes` outputs, never a
  workflow-level `paths:` filter (a required check would wait forever),
  and keep pushes to `main` running everything.

Done when `scootbg daemon` connects, binds its globals, answers `query`
with an empty output list shape, and exits cleanly on `kill` and on
compositor exit, with tests for the socket lifecycle.

## Resolution

### What landed

- **`crates/scootbg-mem`** (`publish = false`, rustix `linux_raw`, no
  `libc` crate). Every `unsafe` block has a `// SAFETY:` argument and
  holds one operation: `undocumented_unsafe_blocks`,
  `multiple_unsafe_ops_per_block` and `missing_safety_doc` are denied in
  its `[lints]`, with `unsafe_op_in_unsafe_fn`.
  - `alloc.rs`: `LargeAlloc`, as designed in §11a of the dependency
    record: ≥ 128 KiB with align ≤ 4096 (a constant, never
    `page_size()`) is its own mapping, `mremap(MAYMOVE)` when both sizes
    are big, copy on a threshold crossing, over-aligned big blocks left to
    `System` with the comment §11 asked for, null on failure.
  - `shm.rs`: `ShmBuffer` / `Attached`, as designed in §11b: `len` and
    `stride` from `checked_mul`, bounded to `i32::MAX`, non-zero; memfd
    sealed `SHRINK | GROW | SEAL`; `attach(self)` consumes the writable
    handle and only `Attached::released` gives it back. `Send`, not
    `Sync`. Not used by the daemon yet (ticket 4 onwards).
  - Nothing else: no signal handling (see "Departures" below).
- **`crates/scootbg`** (`#![forbid(unsafe_code)]`, `LargeAlloc` as
  `#[global_allocator]`), one binary, hand-rolled CLI: `daemon`, `query`,
  `version`, `kill`, `--version`, and `--help` for each. `set`, `clear`
  and `apply-config` are neither parsed nor advertised.
  - The daemon claims the socket, connects with plain `wayland-client`,
    binds `wl_compositor` (v4–6), `wl_shm` and `zwlr_layer_shell_v1`
    (required: missing or too old is an exit 1 naming it) and
    `wp_viewporter`, `wp_single_pixel_buffer_manager_v1`,
    `wp_fractional_scale_manager_v1` (optional: a one-line note each when
    absent), and binds each `wl_output` as it appears, releasing it on
    `global_remove`. No surfaces.
  - One thread, one `poll` over the Wayland fd, the listener and each
    client, with no timeout, and no async runtime or
    timers. The poll set's `Vec` is reused across rounds (a test pins that
    its allocation survives), connection buffers are reused per line, and
    request parsing borrows from the line.
  - Protocol: one JSON object per line each way. Requests carry
    `"protocol": 1` and `"type"`. A missing or different `protocol`, an
    unknown `type` or malformed JSON gets an error reply and the
    connection stays open. Replies are tagged by `type`: `outputs`
    (`query`; the list is empty until ticket 3 gives `OutputEntry` its
    fields), `version`, `ok`, `error`.
  - Bounds: request lines ≤ 64 KiB, refused (error reply, then close)
    as soon as 64 KiB + 1 bytes arrive without a newline; each byte is
    scanned once, so a trickling client costs linear time. At most 16
    clients: the 17th closes the oldest, so stuck clients can never lock
    out `kill`, with no timers. A connection with unsent replies is not
    read (backpressure), and at most ~4 KiB of replies queue per read. At
    most 16 accepts per wakeup.
  - The listener never leaves the poll set. Out of fds (`EMFILE`/`ENFILE`,
    acted on only when a zero-timeout poll shows a client really waiting,
    because Linux reports them before it looks at the queue), the server
    closes the oldest client, else its spare fd (a `dup` of the listener,
    so no path such as `/dev/null` can be missing), and accepts the
    waiting client, so `kill` gets through. With nothing left to close,
    and for any `accept` error that is not about one connection (`ENOMEM`,
    `ENOBUFS`, ...), the daemon exits 1, releasing its lock, rather than
    spin or go deaf. Per-connection errors (`ECONNABORTED`, `EPROTO`, ...)
    are skipped.
  - Exit: `kill` exits 0; a compositor exit, a protocol error or a
    broken connection exit 1; usage errors exit 2. A compositor that
    hangs up with nothing left to read is recognised before
    `wayland-backend` is asked to read it, because without its `log`
    feature (which compiles C, see the Standards in
    [the README](../../README.md)) the backend reports that error with
    `eprintln!`: a stray line on every exit, and a panic when stderr is a
    pipe whose reader has gone. For the backend's other `eprintln!` paths
    (a protocol error, `WAYLAND_DEBUG`), a panic hook removes the socket and
    turns std's "failed printing to stderr" panic into exit 1; any other
    panic still aborts, loudly. scootbg's own writes never panic. Each of those removes
    the socket first, and no panic is reachable from input. Signals keep
    their default action (see "Departures").
  - Clients against no daemon, or a dead daemon's leftover socket (connect
    gets `ENOENT` or `ECONNREFUSED`), print "no scootbg daemon is running
    for NAME … start one with `scootbg daemon`" and exit 1; `kill` too,
    as its `--help` documents.
- **Nix**: `packages.<linux>.scootbg`, built `-p scootbg` (the `scoot`
  package is unchanged). Linux only: on other systems the crate builds a
  stub `main` that says so, and `scootbg-mem` is empty, so `cargo check
  --workspace` still passes on a Mac. The dev shell needed nothing new
  (the toolchain builds it); that is its "place in the dev shell".
  `nix-build.yml` builds and runs the package on main.
- **CI**: the path split in the table above (`changes` now outputs
  `scoot` and `scootbg`), a `scootbg` job (fmt, clippy `-D warnings`,
  nextest and `cargo test` for both crates, a `cargo tree` check that no
  `libc` crate is in scootbg's normal tree, the stripped release size and
  an `ldd` check for anything beyond std's libraries), and a
  `scootbg-integration` job that builds `scoot` and runs
  `crates/scootbg/tests/daemon.rs` with `SCOOTBG_REQUIRE_SCOOT=1`, so it
  cannot pass by skipping. A scootbg-only change skips the macOS
  `cargo check` and `nix flake check` (they live in the scoot jobs), so
  a break there would first show on main, which runs everything. The two
  crates' `Cargo.toml`s are the files those jobs read (`flake.nix` takes
  the package description from one), so a change to either runs
  everything.

### Departures from the plan, and why

- **A lock file decides liveness, not the socket bind.** "Stale socket
  detected and replaced" with only a connect probe races: two daemons
  can both see the socket dead, both remove it, and the later removal
  deletes the earlier daemon's live socket. So a daemon holds an
  exclusive `flock` on `scootbg-NAME.lock` for its life, the way
  libwayland pairs `wayland-N` with `wayland-N.lock`. The lock file is
  never removed (removing it would reopen the race). A connect probe is
  kept as a second check, so a daemon running without the lock (someone
  deleted the file) still gets a refusal rather than its socket stolen.
  **For [scoot-integration-done.md](scoot-integration-done.md):** the loser of
  two racing `apply-config` starts now learns it lost from the lock,
  possibly before the winner has bound. The winner binds straight after
  the lock, before connecting to Wayland, so the window is short, but the
  forwarding `apply-config` must retry its connect briefly.
- **Signals are not caught; they keep their default action.** The
  ticket asked for SIGTERM/SIGINT to exit cleanly and remove the socket,
  through a safe `signalfd` or self-pipe in rustix. **rustix has neither:**
  `signalfd` is in its `not_implemented` list, and its only signal APIs are
  the `unsafe`, `#[doc(hidden)]`, not-semver-stable `rustix::runtime`
  functions (a handler through `kernel_sigaction` would also need a
  per-architecture restorer). The first version of this PR therefore had a
  third `unsafe` module in `scootbg-mem`: it blocked TERM/INT/HUP with
  `kernel_sigprocmask` before any thread existed, and a 64 KiB-stack thread
  `kernel_sigwait`ed and wrote to a pipe the loop polled.
  **The coordinating session removed it**, and this is what landed.
  Cleanup on a signal buys nothing the lock does not already give: the
  kernel drops the `flock` however the process dies, clients get
  `ECONNREFUSED` on the leftover file and say "not running", and the next
  daemon replaces the file. Dropping it removes an `unsafe` module (so
  `scootbg-mem` is back to exactly its two memory modules and its name
  stays accurate), the dependency on a hidden rustix API, a blocked signal
  mask that every future child would inherit, and a thread. The
  integration test `a_signal_kills_the_daemon_and_the_next_one_replaces_its_socket`
  pins it: each of TERM, INT and HUP kills the process by that signal, the
  lock is free straight after, `query`/`version`/`kill` report not running
  with exit 1, and the next daemon serves. If clean signal exits are ever
  wanted (a session manager that minds the stale file, say), the options
  are the hidden rustix API as above, or a rustix release with `signalfd`.
- **`kill` releases the lock before closing its clients**, so `scootbg
  kill && scootbg daemon` cannot be refused by a daemon still exiting. A
  second, concurrent `kill` whose connection is dropped unanswered as the
  daemon exits counts as success once the socket is gone, since the stop
  it asked for happened.
- **Replies are printed as the daemon sent them**, one compact JSON line,
  not pretty-printed like `scootctl`'s: `serde_json::Value` would reorder
  the keys, and one line is what scripts want.
- **`scootbg --version`** (local) and **`scootbg version`** (asks the
  daemon) are both there, the same split `scootctl` makes.

### Review of PR #268

Fixed before merge, each with a test:

- **Deaf daemon (blocking).** The listener used to leave the poll set
  whenever the spare fd, then `File::open("/dev/null")`, could not be
  had. In a mount namespace with an empty `/dev`, `scootbg version` timed
  out and `kill` gave up after 30 s. Now the spare is a dup of the
  listener and the listener is always polled (above). Writing the
  `RLIMIT_NOFILE` test turned up a second bug: Linux's `accept` returns
  `EMFILE` before looking at the queue, so the server evicted the client
  it had just admitted. Hence the zero-timeout poll. Getting that
  test right took three tries, all failing on the CI runner, which hands
  its children more fds than a developer shell:
  1. A fixed limit at spawn (`prlimit --nofile=7:7`): the daemon inherited
     more and hit it at start-up (its clean exit 1 there was correct).
  2. The running daemon's limit set to its open fd *count*. That rests on a
     false premise: `RLIMIT_NOFILE` bounds fd *numbers* (a new fd takes the
     lowest free number, and only if it is below the limit), not how many
     are open. With inherited fds at high numbers there were free numbers
     below the limit, accepts succeeded into them, and "open ≤ limit"
     failed. That is deterministic, not a race: two non-contiguous
     inherited fds (`exec 10</dev/null 11</dev/null`) reproduce it every
     time, and contiguous ones never do, which is why a first harness
     passed. The claim that it "reached exhaustion however many fds the
     environment hands down" was wrong.
  3. Now: wait until the daemon's fd set is stable, then set its limit to
     its lowest *free* fd number (fds already open above that stay open,
     which is legal). Every new fd then fails with `EMFILE` whatever the
     numbering. Exhaustion is proven by behaviour (with five idle clients
     at least four are evicted, against none without the limit, far below
     the 16-client cap) and by fd numbers (nothing opened after the limit
     is at or above it), not by an open-count bound. Proven ×50 per
     configuration, 0 failures: non-contiguous inherited fds at 0, 2, 20
     and 200, contiguous ones at 2, 20 and 200, and the non-contiguous
     ones again under four CPU hogs. strace shows 14 `accept4 … EMFILE`
     in each configuration.

  The test fails rather than skips without `prlimit` under
  `SCOOTBG_REQUIRE_SCOOT`.
- **Crash hook disarmed before release.** The panic hook used to stay
  armed after `claim.release()`, so a panic during shutdown (closing
  clients, dropping the Wayland connection), after a new daemon had
  taken the lock and bound the path, would have removed the new daemon's
  socket. It is disarmed first now, and removes at most once.
- **Flaky tests.** Two connection tests bounded their waits by an
  iteration count, not the clock, and failed under a loaded `cargo test`.
- **Broken stderr → abort.** Above, under Exit.
- **Overstated safety comments** in `shm.rs`: the compositor maps the
  pool from `create_pool` on, not from `attach`, so `pixels_mut`'s
  soundness rests on the compositor not writing client buffers
  (`wl_shm`'s contract, not a kernel guarantee), which the comment now
  says; the type state is described as a discipline for scootbg's own
  code, not a guarantee.
- **Nits:** a request must be a JSON object (`[1,"kill"]` used to parse
  and stop the daemon); the stale-socket probe connects non-blocking
  (a stopped listener with a full backlog hung start-up); a line buffer
  never grows past 64 KiB + 1, below the allocator's threshold; the CI
  path split above; exit status 2 in the README.

### Revisit in ticket 3

Harmless while the daemon draws nothing, user-visible (a wallpaper that
vanishes) once [outputs-and-layer-surfaces-done.md](outputs-and-layer-surfaces-done.md)
gives it surfaces:

- **Exit 1 on transient resource errors.** An `accept` failing with
  `ENFILE`, `ENOMEM` or `ENOBUFS` while there is nothing to free ends the
  daemon (above), which is right for a daemon that only answers a socket
  but would take the wallpaper with it. Weigh keeping the surfaces up
  and only shedding the listener until a client closes, or a restart
  path, against the spin and deafness this rule exists to prevent.
- **Starvation at the fd limit.** A same-uid process flooding connects
  while the daemon sits at its fd limit keeps evicting clients, so a
  legitimate `scootbg` command can fail (fast, without a spin). Same uid
  can already `kill` the daemon, so this is not a privilege problem, but
  a wallpaper change lost to it would be visible.

### Left for later tickets

- Output tracking, `OutputEntry`'s fields, and layer surfaces:
  [outputs-and-layer-surfaces-done.md](outputs-and-layer-surfaces-done.md).
  Outputs are bound here, but their events are ignored.
- `set`, `clear`, `apply-config`, and the `--output`/`--mode`/`--fill`
  flags: [cli-and-ipc-done.md](cli-and-ipc-done.md).
- `--profile` and `--no-restore`: [restore-state-done.md](restore-state-done.md).
- The first real use of `ShmBuffer`: [solid-color-done.md](solid-color-done.md)
  (its 1×1 fallback) and [images-decode-and-fit-done.md](images-decode-and-fit-done.md).
- Competitor measurements: [lightest.md](../lightest.md).
- **Not verified here:** `--tty` (no dev VM was reachable from the Claude
  Code web container this was built in; nothing in this ticket touches
  it), another layer-shell compositor (ticket 3 names that), and the
  macOS `cargo check` (no Darwin toolchain here; CI's macOS job runs it on
  this PR).

### Measurements

A Claude Code web container (x86_64, rustc 1.97.1 from the devenv
shell), `scoot --headless --outputs 2` in a scratch `XDG_RUNTIME_DIR`,
release profile, `cargo build --release -p scootbg`:

| What | Result |
|---|---|
| Stripped binary | 672,480 bytes after review (664,288 before it, 680,776 with the signal thread); +8 KB is rustix `net` (the non-blocking probe), the fd-exhaustion handling and the crash hook. Nix package 669,032 |
| `ldd` | `libc.so.6`, `libgcc_s.so.1` (not even `libm`) |
| `libc` crate in the normal tree | none; rustix build output `cargo:rustc-cfg=linux_raw` |
| `-sys` crates | `linux-raw-sys`, and `wayland-sys` built with no features |
| Idle RSS / PSS / heap, 3 s after start, 3 runs | 2,576 / 1,380 / 164, 2,580 / 1,384 / 164, 2,600 / 1,404 / 164 kB; 1 thread (with the signal thread, interleaved: 2,608–2,624 / 1,412–1,428 / 172–176 kB, 2 threads) |
| Idle context switches, 3 × 30 s | 0, 0, 0 (with the signal thread: 0, 0, 0) |
| After review, interleaved with the pre-review binary, 3 rounds | RSS 2,564 / 2,572 / 2,588 kB, PSS 1,368 / 1,376 / 1,392, heap 164, 1 thread, 7 fds (stdio, lock, listener, spare, Wayland), always, idle or serving: a normal accept never touches the spare, which is spent only at the fd limit (`EMFILE`/`ENFILE` with a client waiting), so 6 appears only there, 0 switches and 0 ticks per 30 s (pre-review: 2,588 / 2,600 / 2,568, 1,392 / 1,404 / 1,372, 160–164; a tie); `query` round trip median 27.2 / 26.2 µs against 26.4 / 26.2 |
| Idle CPU ticks, 3 × 30 s | 0, 0, 0 |
| `query` round trip on one connection, 3 × 10,000 (with the signal thread; the request path is unchanged) | median 27.9 / 26.4 / 26.6 µs, p99 63.3 / 57.7 / 62.3 µs |
| `scootbg query` as a command, 3 × 100 | 1,654 / 1,688 / 1,636 µs each, process start included |
| Memory after those ~30,500 requests | RSS and heap unchanged (measured with the signal thread) |

The prototype in the dependency record (§1, prototype A) idled at 2,500
kB RSS and 164 kB heap with no socket or allocator wrapper.
