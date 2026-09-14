---
item: "9"
title: "IPC hardening: permissions, peer credentials, bounded requests"
status: "done"
area: "ipc"
pr: 14
commit: null
---

# IPC hardening: permissions, peer credentials, bounded requests

Context that shaped the scope: sway's own `ipc-server.c` (read, not assumed)
does no `chmod` and no peer check either — it relies entirely on
`$XDG_RUNTIME_DIR` being `0700`, as do i3, niri and Hyprland. So flexwm was
not behind anyone architecturally. What makes the asymmetry worth closing
anyway is that flexwm's IPC injects arbitrary input and returns
screenshots, where theirs mostly reads state and runs layout commands — so
the blast radius if the socket is ever reachable by someone else is bigger
than the norm this matched.

- **Owner-only socket.** `ipc/listener.rs` sets the socket `0600`. Not
  theoretical: with `umask 000` the pre-change binary published
  `srwxrwxrwx` and another local user could both read state *and* inject
  keystrokes (`flexwm msg type` as `nobody` returned `Ok`); after, the mode
  is `0600` whatever the umask and that same call gets `EACCES`. Under the
  ordinary `umask 022` the socket came out `srwxr-xr-x`, which already
  refused other users (connecting needs write permission), so the exposure
  was umask-dependent, not unconditional — the Backlog entry's "non-default
  umask" half was the real one.
- **Same-user peer check.** `accept()` compares the peer's uid against the
  compositor's own before the connection costs an fd, a buffer or a place
  in the event loop. This is a second, independent layer, proven
  independent on hardware: root bypasses the file mode entirely, connects,
  and is refused by the uid check (`Connection reset by peer`, with a
  `warn!` naming uid 0). An exact match, so root is refused too — it can
  reach the process by other means anyway.
  - **Effective, not real, uid.** Linux fills `SO_PEERCRED` from the
    peer's *euid* (`cred_to_ucred` uses `cred->euid`), so the comparison
    is against `geteuid()`; using `getuid()` would compare two different
    things in the one case they differ.
  - Two claims in the plan for this item turned out wrong in the code and
    were corrected rather than worked around: `UnixStream::peer_cred` is
    **not** stable (still `peer_credentials_unix_socket`, rust#42839 — the
    build failed on it), and rustix's `socket_peercred`, though rustix is
    already a dependency, reads the kernel's `struct ucred` straight into a
    `UCred` whose `pid` is a `NonZeroI32` — and the kernel writes pid 0
    for a peer in a PID namespace this process can't see it in
    (`pid_vnr`), a niche-invalid value that declining to read the field
    does not avoid. So the sockopt is asked for by hand through `libc`
    (already in the tree; a direct dependency, not a new crate), reading
    only the uid, with the struct pre-filled with the kernel's own `-1`
    sentinel so an unwritten one can never read back as a valid uid.
    `geteuid` has no such hazard and still goes through rustix's safe
    wrapper.
- **Bounded request line.** `Connection::step` read through
  `BufRead::read_line` into an unbounded buffer, so one connection
  streaming bytes with no `\n` grew it without limit. Replaced with
  `ipc/line.rs`'s explicit `fill_buf`/`consume` loop and a 1 MiB cap
  (generous: the largest real request is `Request::Type`'s text, and a
  500 KB one still works). Measured, release builds, 200 MiB of
  newline-less garbage from one connection: before, compositor RSS
  10,540 kB → 215,344 kB (VmPeak 282,620 kB) and still waiting; after,
  the client's write dies with `EPIPE` at 1,114,112 bytes, gets an
  explicit error reply, and RSS stays at 10,736 kB (VmPeak 21,512 kB).
  `flexwm-ipc`'s shared `read_message` is deliberately **not** capped: the
  same codec reads `Response::Screenshot`, legitimately several MB of
  base64 PNG, so a global cap there would break real screenshots.
- **Per-connection screenshot rate limiting.** A capture is a full render
  plus framebuffer read-back plus PNG encode on the one event-loop thread.
  A connection handed one less than one `FRAME_INTERVAL` (16ms, reused
  from `headless.rs`, not a new magic number) ago now gets a
  `Response::error` instead of another capture. Never a sleep — that would
  block every other client to slow one down; a refusal is answered in
  microseconds.
  **The first implementation stamped the clock when the request arrived
  and was a complete no-op, caught by bug-bashing it on hardware rather
  than by any test**: a capture takes longer than a frame (~170ms for
  800x600 in a debug build, ~12ms for 1600x1000 in release), so by the
  time the next request arrived the window had always already expired —
  50 back-to-back requests, 50 served, 0 refused. Stamping when the
  capture *finishes* is what makes the window mean anything, and is what
  the ticket actually wanted ("leave the event loop room after a
  capture"). Release, 1600x1000, 200 back-to-back requests on one
  connection: before 200 served in 2.45s costing 242 compositor jiffies;
  after 2 served / 198 refused in 48ms costing 2 jiffies. Per connection,
  so bypassable by reconnecting per capture — capping concurrent
  connections is the audit's separate finding and stayed out of scope.
- **Atomic publish instead of unlink-then-bind.** `listener::bind` creates
  a `0700` directory beside the socket path with `mkdir`, opens it
  `O_DIRECTORY | O_NOFOLLOW`, binds the socket inside it as
  `/proc/self/fd/<n>/s`, chmods it `0600` there, and `rename`s it onto the
  published path, removing the directory either way. The Backlog called the
  old order a symlink race; tracing it showed that is not the exposure —
  `bind(2)` does not follow a symlink at the final component, confirmed
  empirically (a dangling symlink at the path makes bind fail `EADDRINUSE`,
  errno 98), and `remove_file` unlinks a link rather than its target. What
  it really had was a window where the socket existed at its published name
  with whatever the umask allowed before the `chmod` could run, plus a
  window where the name was missing and another process could claim it.

  This took three attempts, the first two of which `flexwm-reviewer` and a
  self-review caught as *worse* than what they replaced — worth recording,
  because each one failed for the same reason: a path that another user can
  write to cannot be re-resolved by name once it has been checked.
  (a) Binding at `<path>.<pid>.tmp` and chmodding that path:
  `set_permissions` follows symlinks, so they could swap the staged socket
  for a link between the bind and the chmod and have the compositor chmod a
  file of their choosing. (b) Staging inside `<path>.<pid>.tmp/` but
  *pre-cleaning* that name first: `remove_file` on `<staging>/socket`
  resolves `<staging>` as a non-final component, so a symlink planted there
  — cheaply, per-pid, in advance — deleted a file of their choosing through
  it, and then `mkdir` failed `EEXIST` against the surviving link on every
  subsequent start. (b) also cost 17 path bytes, which broke binding for
  any path over ~90 bytes, at a threshold that moved with the pid's digit
  count. The third shape fixes both classes at once: `mkdir` *is* the claim
  (it neither follows a symlink nor replaces a name, so nothing is ever
  removed to make room), an unpredictable `.flexwm-<12 random>` name means
  there is nothing to pre-plant at, everything after the claim goes through
  the pinned fd rather than the name, and `/proc/self/fd/<n>/s` is both
  unswappable and a fixed ~17 bytes regardless of the published path — so a
  107-byte socket path, the longest `sun_path` allows, binds. All three
  primitives were verified on the dev VM before the code was written, not
  assumed: `O_DIRECTORY | O_NOFOLLOW` against a symlink-to-a-directory
  fails `ENOTDIR`, `File::set_permissions` fchmods a read-only directory fd,
  and `rename` out of `/proc/self/fd/<n>/` onto a 107-byte path works and
  stays connectable.

  The umask is the other way to get a `0600` socket with no chmod at all,
  and was the second attempt's successor before being abandoned mid-flight:
  it is process-global, and the test suite proved that is not academic —
  `tempfile::tempdir()` in a *concurrent* test, created inside the umask
  window, came out mode `0600`, which for a directory means untraversable,
  and unrelated tests failed with `EACCES` in one run out of three. A
  hazard that live in-process is not something to ship behind a comment.

**Owner-only socket, restated for the final shape:** the mode is set on
the socket while it is still inside the staging directory, so it is `0600`
before it is reachable under its published name at all — there is no window
at the published path, whatever the umask.

`ipc.rs` split into `ipc/line.rs`, `ipc/listener.rs` and `ipc/tests.rs`
before it sprawled. 128 tests (26 new, against the 102 on the merge
base), clippy/fmt clean, `cargo test`
green workspace-wide, `scripts/smoke-test.sh` green under both `--headless`
and `--nested`, and the whole bug-bash re-run against real `--tty` hardware
(it is IPC-layer code, so it is backend-agnostic by construction — but
confirmed, not assumed). Per-request cost of the bounded read, measured
because it is on the per-request path: release, 50,000 `version`
round-trips, 6 interleaved
reps per side — before mean 122.42us/66.5 jiffies, after mean
122.03us/66.0 jiffies, fully overlapping. (Debug builds showed a consistent
~5% jiffies gap, which is a debug-build artifact: std's `read_line` uses
`memchr` where this loop uses a plain byte scan, and only the unoptimized
build can tell.)

**`flexwm-reviewer`'s pass found two blocking regressions, both in
`listener::bind`, both fixed as described above** — and both in the part of
the change that was *new* rather than in the four audit fixes themselves,
which it re-derived and confirmed (`geteuid` over `getuid`, `libc` over std
or rustix, completion-stamped throttling, the 1 MiB cap and the peer check
under adversarial testing, no measurable benchmark regression). It also
found three smaller things, all addressed: the throttle's justification
overclaimed in two places (`FRAME_INTERVAL`'s doc and the client-facing
error both said the screen "cannot have changed", which `render()`'s
on-demand scheduling does not guarantee — reworded to the claim that is
true, that it bounds what one connection can cost the event loop), this
entry and the PR description still described the superseded design, and the
deferred blocking-I/O finding below was under-rated at MEDIUM.

**Three pre-existing defects found while bug-bashing this, deliberately not
fixed here** — all verified identical on the pre-change binary, so none is a
regression, and all are one Backlog entry below (closed as item 10): a
half-written request line
blocks the entire event loop for as long as the client holds it (every other
client included), a second request pipelined into the same write is never
answered, and a client that never reads its replies deadlocks the loop from
the write side. One root cause (the connection does blocking I/O and reads
one line per readiness event), one fix, and that fix restructures the
connection loop — its own item, not a rider on this one.
