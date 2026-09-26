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
    most 16 accepts per wakeup. If the process ever runs out of fds, a
    spare fd is spent to turn the waiting client away rather than leave
    the listener readable and spin.
  - Exit: `kill` exits 0; a compositor exit, a protocol error or a
    broken connection exit 1; usage errors exit 2. Each of those removes
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
  cannot pass by skipping.

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
  **For [scoot-integration.md](../scoot-integration.md):** the loser of
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

### Left for later tickets

- Output tracking, `OutputEntry`'s fields, and layer surfaces:
  [outputs-and-layer-surfaces.md](../outputs-and-layer-surfaces.md).
  Outputs are bound here, but their events are ignored.
- `set`, `clear`, `apply-config`, and the `--output`/`--mode`/`--fill`
  flags: [cli-and-ipc.md](../cli-and-ipc.md).
- `--profile` and `--no-restore`: [restore-state.md](../restore-state.md).
- The first real use of `ShmBuffer`: [solid-colour.md](../solid-colour.md)
  (its 1×1 fallback) and [images-decode-and-fit.md](../images-decode-and-fit.md).
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
| Stripped binary | 664,288 bytes (680,776 with the signal thread) |
| `ldd` | `libc.so.6`, `libgcc_s.so.1` (not even `libm`) |
| `libc` crate in the normal tree | none; rustix build output `cargo:rustc-cfg=linux_raw` |
| `-sys` crates | `linux-raw-sys`, and `wayland-sys` built with no features |
| Idle RSS / PSS / heap, 3 s after start, 3 runs | 2,576 / 1,380 / 164, 2,580 / 1,384 / 164, 2,600 / 1,404 / 164 kB; 1 thread (with the signal thread, interleaved: 2,608–2,624 / 1,412–1,428 / 172–176 kB, 2 threads) |
| Idle context switches, 3 × 30 s | 0, 0, 0 (with the signal thread: 0, 0, 0) |
| Idle CPU ticks, 3 × 30 s | 0, 0, 0 |
| `query` round trip on one connection, 3 × 10,000 (with the signal thread; the request path is unchanged) | median 27.9 / 26.4 / 26.6 µs, p99 63.3 / 57.7 / 62.3 µs |
| `scootbg query` as a command, 3 × 100 | 1,654 / 1,688 / 1,636 µs each, process start included |
| Memory after those ~30,500 requests | RSS and heap unchanged (measured with the signal thread) |

The prototype in the dependency record (§1, prototype A) idled at 2,500
kB RSS and 164 kB heap with no socket or allocator wrapper.
