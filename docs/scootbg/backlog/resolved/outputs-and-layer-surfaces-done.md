---
title: "One background layer surface per output, across hotplug"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# One background layer surface per output, across hotplug — RESOLVED

Resolved 2026-09-27. What landed, where it departs from the plan, what
was verified where (and what could not be), and the measurements are in
[Resolution](#resolution) at the end; the original ticket follows
unchanged.

- Track `wl_output`s (name, description, mode, scale) as they appear and
  vanish; bind `xdg-output` only if `wl_output` v4's `name` is missing.
- Per output: a `zwlr_layer_surface_v1` on the `background` layer,
  namespace `wallpaper`, anchored to all four edges, exclusive zone `-1`,
  keyboard interactivity `none`, and an empty input region so pointer
  events fall through.
- Handle `configure` sizes of 0 (use the output's size), a `closed` event
  (the compositor removed the surface: recreate once, then give up loudly),
  and an output removed while an image is still decoding for it (drop the
  result, do not panic).
- Zero outputs is a normal state (a headless session before its first
  output, a laptop with the lid shut): the daemon idles and waits.

Test on scoot `--headless --outputs 2` (both outputs covered, the second
checked by `scootctl screenshot --output 2`), and on at least one other
layer-shell compositor before calling it portable.

## Resolution

### What landed

- **`src/outputs.rs`, a pure model** of each output and its surface, with
  no Wayland objects and no I/O, so every event ordering is a unit test
  (`src/outputs/tests.rs`, 20 tests). Per output: its registry name, an
  `OutputId` never reused for the daemon's life, properties *staged* until
  `wl_output.done` applies them (name, description, current mode, scale,
  transform, and `xdg_output`'s logical size), and the surface state:
  `Waiting` → `Pending` (created, committed with no buffer) →
  `Configured { serial, requested }` → on `closed`, `Closed` (destroyed,
  re-created once after a round trip) or, closed again, `GaveUp`. Events
  return an `Effect` (`Create`, `Ack`, `DestroyAndRetry`,
  `DestroyAndGiveUp`) that the glue carries out. Nonsense from the
  compositor (a mode or logical size ≤ 0, a scale < 1) is ignored rather
  than let it reach a size.
- **`src/daemon/surfaces.rs`, the Wayland glue.** Each object carries its
  output's `OutputId` as user data, and every event looks the output up
  again, so anything for a removed output is dropped whatever it was.
  - The surface: `background` layer, namespace `wallpaper`, anchored to all
    four edges with size 0×0, exclusive zone -1, keyboard interactivity
    `none`, and an input region that is a `wl_region` with nothing added.
    Committed once with no buffer; each `configure` is acked at once.
    Nothing is attached, so nothing is shown; solid-colour.md attaches to
    a `Configured` surface and commits.
  - Removal destroys the layer surface, then its `wl_surface`, then the
    `xdg_output` if any, then `wl_output.release` (v3+).
  - `zxdg_output_manager_v1` is bound only when an output older than
    `wl_output` v4 (which has no `name`) appears, and then once.
- **`query`** lists every output (additive: the reply was always a list,
  and the protocol version stays 1):

  ```text
  {"name":"DP-1","description":"...","mode":{"width":3840,"height":2160},
   "scale":2,"transform":"normal","logical":{"width":1920,"height":1080},
   "surface":{"state":"configured","size":{"width":1920,"height":1080}},
   "shows":null}
  ```

  Every key is always there, `null` when not known. `state` is `waiting`,
  `pending`, `configured`, `closed` or `gave-up`; `size` only when
  configured. `shows` is `null` until solid-colour.md. The entries borrow
  from the model and serialize straight into the connection's reused
  buffer (a unit test pins that the buffer is not reallocated).
- **Zero outputs** is the same poll with no timeout: no wakeups, measured
  (below) and tested (`hotplug.rs`).
- **The two items carried from ticket 2** ("Revisit in ticket 3" in
  [crate-and-daemon-done.md](crate-and-daemon-done.md#revisit-in-ticket-3)):
  - *An unrecoverable `accept` no longer ends the daemon.* It used to exit
    1, which would now take the wallpaper with it. Instead the listener
    **rests** (`src/daemon/listen.rs`): it stays in the poll set with no
    events asked for (so a connection it cannot take does not keep a
    level-triggered `poll` returning), the poll gets a 1 s timeout, and the
    listener is re-armed and tried again when it runs out. A persistent
    failure is one wakeup a second, never a spin and never deaf for longer
    than a second; the timeout exists only in that state, so a normal idle
    daemon still makes no wakeups. The failure is reported once, and its
    end ("accepting clients again") once. Unit tests drive the state with
    an injected clock, and an integration test reproduces it for real (see
    below).
  - *Starvation at the fd limit is documented, not fixed* (`control`'s
    module docs): a same-uid process flooding connects at the fd limit
    keeps evicting clients, so a legitimate command can fail fast; that
    user can already `kill` the daemon or set the wallpaper, and every way
    to tell the flooder from the legitimate client (peer credentials, a
    quota, a priority) finds the same uid on both. The daemon and its
    wallpaper stay up either way.

### Departures from the plan, and why

- **An output's surface is created when a `wl_display.sync` sent right after
  the bind comes back, not at the first `done`.** Two reasons, both seen
  on real compositors. A `wl_output` older than v4 gets its name from
  `xdg_output`, whose events arrive *after* the output's first `done` (a
  trace with a build capping `wl_output` at v3 on sway shows
  `wl_output.done`, then `zxdg_output_v1.name`, then `done` again). And a
  v1 `wl_output` never sends `done` at all. Every event the compositor
  sends in answer to the bind and to `get_xdg_output` precedes the sync's
  reply, so at that point the identity is as known as it will be. An
  output that never sent `done` is settled with what it did send.
- **After `closed`, the surface is re-created one round trip later, not
  at once.** sway closes an output's layer surfaces and then withdraws the
  output, in one batch (trace: `closed`, then `global_remove`). Re-creating
  at once would ask for a surface on an output that is going away, which
  compositors hand to *another* output (sway picks the focused one, scoot
  the primary). A `wl_display.sync` after the `closed` lets any removal
  arrive first; the retry then finds the output gone and does nothing. The
  "creating it again" message is printed only when a retry really happens,
  so an unplug says nothing.
- **`query`'s `logical` is the best size known, not only the one derived
  from `wl_output`.** At a fractional scale `wl_output` reports the scale
  rounded up, so mode ÷ scale is too small: scoot at 1.5 on 1600×1000
  says scale 2, which derives 800×500, while the real logical size is
  1067×667. So `logical` is the configured surface's size once there is
  one (the surface covers the whole output), else `xdg_output`'s, else the
  derivation. The 0-size `configure` fallback uses the latter two, as the
  ticket asked, and says in code that it undershoots at fractional scales
  and is `None` (the surface waits) before any mode is known. No
  compositor checked sends 0 for a surface anchored to all four edges.
- **No commit after the ack.** An ack takes effect with the next commit,
  and with no buffer to attach there is nothing to commit; the first
  commit is solid-colour.md's, with its buffer. Several acks before one
  commit are allowed ("only the last … indicates which configure event the
  client really is responding to").
- **The ticket's "second output checked by `scootctl screenshot --output
  2`" moves to solid-colour.md**: with nothing drawn, a screenshot shows
  only scoot's background colour. What stands in for it here is scoot's
  own `wl_surface.enter`: each surface is entered on exactly the output it
  was made for (`outputs.rs`, from the protocol trace), and `usable` is
  unchanged (exclusive zone -1 reserves nothing). scoot has no IPC request
  that lists layer surfaces (`docs/ipc.md`).
- **`src/output.rs` is now `src/print.rs`.** It holds the panic-free
  stdout/stderr writers, and `output` beside the new `outputs` would have
  read as the same thing. A rename, no change.

### Found along the way

- **`poll` fails with `EINVAL` when the process's fd limit is below the
  number of fds polled** (poll(2): "nfds exceeds the RLIMIT_NOFILE
  value"), and the daemon exits 1 on a failed poll. The first version of
  the resting test set the daemon's soft limit to 0 with `prlimit` and hit
  exactly that. The daemon cannot get there by itself: every polled client
  is an fd numbered below the limit, and it holds at least seven fds, so
  the poll set is always smaller than the limit. Only lowering a running
  daemon's limit from outside below its poll set does it. Left as is; the
  test now sets the limit to the spare's fd number instead (below).
- **A flaky test from ticket 2**, `a_stale_socket_is_replaced`, failed
  once in 20 `cargo test` runs at 16 threads: it fakes a stale socket by
  binding and dropping a listener, and another test's `fork` in the same
  process can hold a copy of that fd until its `exec` closes it, so the
  daemon's probe connects and (correctly) refuses. The test now waits for
  the socket to refuse connections, after which no copy exists. 0 in 40
  runs since.
- **sway's `transform 90` turns clockwise; `wl_output`'s transforms count
  counter-clockwise**, so sway reports it as `270`. `query` reports the
  protocol's value, and its docs say so.

### Verified where

All on a Claude Code web container (x86_64, 4 CPUs), evidence against the
working tree on top of `87e1935` that became this PR's commit.

- **scoot `--headless`, 1 and 2 outputs** (`tests/outputs.rs`): both
  surfaces configured at the outputs' 1600×1000; `usable` equals `rect`;
  the trace shows layer 0, namespace `wallpaper`, anchor 15, size 0×0,
  exclusive zone -1, keyboard interactivity 0, an input region with only a
  `destroy` request, no `attach`, and scoot's `enter` on the right output
  for each surface. **Fractional scale** (`scale = 1.5`): `wl_output`
  scale 2, surface and `logical` 1067×667, matching scoot's `rect`; a live
  change to 2.0 through `reload` reconfigures to 800×500.
- **sway 1.12 headless** (wlroots 0.20.2, pixman, from the pinned
  nixpkgs; `tests/hotplug.rs`): an added output gets a surface configured
  to sway's size for it; an unplugged one's surface is destroyed and its
  `wl_output` released; down to zero outputs the daemon serves and idles
  (no context switches over 1.5 s); an output after that is covered;
  scale 2 and transform 90 reconfigure; twenty more add/unplug cycles
  leave the settled fd count unchanged; and the trace balances: 23
  outputs bound, 22 released, 23 surfaces made, 22 destroyed (layer
  surface and `wl_surface` each), `zxdg_output_manager_v1` advertised and
  never bound, and no "creating it again", "giving up" or "cannot accept"
  line. A daemon started with no outputs waits and covers the first.
- **The xdg-output path**, which neither compositor needs (both have
  `wl_output` v4): a temporary build capping `wl_output` at v3, run by
  hand on sway, bound the manager once, named both outputs from
  `zxdg_output_v1.name` (`HEADLESS-1`, `HEADLESS-2`), configured them, and
  on unplug sent `zxdg_output_v1.destroy` then `wl_output.release`. The
  cap was reverted; it is not in the PR.
- **The resting listener, for real** (`an_accept_that_cannot_succeed_…`):
  the daemon's soft fd limit is lowered to its spare's fd number, so every
  number below is taken and freeing the spare does not help; a client
  connects; the daemon reports "cannot accept clients" once, uses ≤ 2 CPU
  ticks in 3 s, stays up with its surface configured; the limit goes back
  and the waiting client is answered within 5 s (about one rest). Run
  against the old behaviour (exit on the error) the test fails.
- **Mutation checks**: skipping the layer-surface destroy on the `closed`
  path fails the hotplug test's trace balance. Skipping it on the
  *removal* path does **not**: on both compositors a removal is always
  preceded by `closed`, so that destroy never has a live surface to act on.
- **Stress**: nextest `--stress-count 20 -j 16` over all of `-p scootbg`
  (107 tests, scoot and sway required): 20/20, idle and again under eight
  busy loops (load average ~10 on 4 CPUs). `cargo test -p scootbg --
  --test-threads 16` × 20: 20/20 idle and 20/20 under the same load.

### Not verified, and why

- **The removal path with a live surface** (a `global_remove` with no
  `closed` before it): neither compositor available does that, and
  scoot's headless backend cannot remove an output at runtime at all. The
  order (layer surface, `wl_surface`, `xdg_output`, `wl_output.release`)
  is by reading, and the model's removal at every state is unit-tested.
- **A real `closed` followed by a re-create or a give-up**: no compositor
  here closes a background surface on an output it keeps. Unit-tested
  (`closed_is_retried_once_then_given_up`, `closed_while_pending_counts_too`,
  `giving_up_on_one_output_leaves_the_others_alone`).
- **`--tty`**, the dev VM: not reachable from this container; nothing
  here touches it.
- **The new CI step** (sway from `nix build --inputs-from . nixpkgs#sway`
  on the runner): the same command resolves to the same store path here
  (`/nix/store/5ddkfdxnq991rfzn2f6n5w1kd6dvaqp3-sway-1.12`) and the tests
  pass with it, but the GitHub runner itself is first exercised by this
  PR's CI run.
- **macOS `cargo check`**: no Darwin toolchain here; CI runs it.

### For the next tickets

- [solid-colour.md](../solid-colour.md): attach to a surface in
  `Surface::Configured` (its serial is already acked) at
  `Output::surface_size()`, commit, and fill `query`'s `shows`. A
  `configure` arriving later (a mode change) needs a redraw at the new
  size. The screenshot check of output 2 lives there now.
- [images-decode-and-fit.md](../images-decode-and-fit.md): a decode result
  carries the `OutputId` it was for and looks it up with
  `Outputs::get_mut` when it lands; an output removed meanwhile is not
  found and the result is dropped. Ids are never reused, so a replugged
  monitor under its old name cannot receive a stale result.
- [hidpi-fractional-scale.md](../hidpi-fractional-scale.md):
  `wl_surface.preferred_buffer_scale` and `preferred_buffer_transform` are
  ignored for now, and `wp_fractional_scale_v1` is bound but not used per
  surface; `logical` falls back to an integer-scale estimate that
  undershoots at fractional scales until the surface is configured.

### Measurements

Release, `cargo build --release -p scootbg`; before is `87e1935` (main)
built from a separate worktree and target dir; against
`scoot --headless --outputs 2` (debug) unless said.

| What | Before | After |
|---|---|---|
| Stripped binary | 672,480 B | 729,824 B (+57,344). The `unstable` feature alone (for `xdg_output`) adds 0 B. About 44 KB is text, all Wayland dispatch instantiated per interface: the effect handler with surface creation inlined (7.2 KB), the `wl_output` event callback (5.6 KB), output binding (5.2 KB), `xdg_output` (≈5 KB), the `wl_surface` request path (3.7 KB) |
| `ldd` | — | `libgcc_s.so.1`, `libc.so.6` (unchanged) |
| `libc` crate in the normal tree | none | none; `-sys`: `linux-raw-sys`, `wayland-sys` with no features (unchanged) |
| Idle RSS / PSS, 3 s after start, 3 rounds interleaved | 2,604 / 1,408, 2,600 / 1,407, 2,584 / 1,388 kB | 2,620 / 1,427, 2,592 / 1,422, 2,632 / 1,436 kB |
| `[heap]` Rss | 28 kB ×3 | 32 kB ×3 |
| Threads, fds | 1, 7 | 1, 7 |
| Context switches, CPU ticks, 30 s idle ×3 | 0, 0 ×3 | 0, 0 ×3 (both surfaces configured) |
| `query` round trip, one connection, 10,000 ×3, two rounds (Python client) | median 25.9–26.3 µs, p99 53.5–59.9 µs, 32-byte reply | median 29.1–38.4 µs, p99 66.7–75.8 µs, 541-byte reply (two entries) |
| RSS after 60,000 queries | unchanged | unchanged |
| 100 add/unplug cycles on sway, release, ×3: RSS / PSS / heap Rss / heap size / fds, before → after | — | 2,668 / 1,475 / 36 / 132 kB / 7 → identical; 2,648 / 1,455 / 36 / 132 / 7 → identical; 2,668 / 1,472 / 36 / 132 / 7 → identical; no stderr, exit 0 |

"Heap" here is the `[heap]` mapping's resident pages (`Rss` in
`/proc/PID/smaps`); the mapping's size is 132 kB for both builds. The
ticket-2 record's "heap 164 kB" was measured some other way and is not
comparable with this row: both builds are compared here by one method.
