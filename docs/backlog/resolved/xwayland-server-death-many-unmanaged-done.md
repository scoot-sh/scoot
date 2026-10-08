---
title: "XWayland dies with ~250 override-redirect windows mapped — RESOLVED (scoot was killing it)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland dies with ~250 override-redirect windows mapped — RESOLVED

RESOLVED 2026-09-27. **It was not XWayland dying. scoot was disconnecting
it.** The X server is one Wayland client carrying every X client's windows,
and scoot held it to the per-client bounds sized for one app: 512 fds in
the fd ledger (`client_fds.rs`) and 512 live `wl_buffer`s
(`wl_buffers.rs`). Each mapped X window costs the server 2 of each, so the
257th window's `wl_shm.create_pool` was refused with a protocol error. That
killed the server and every X window in the session with it. The fix gives
the server its own budget (`compositor/xwayland_budget.rs`): a sixteenth of
the fd table, 512..=4096. That is 4096 on the table scoot raises to, and
exactly the old 512 on a 1024-fd table, so every fd-pressure margin derived
for that table still holds. Measured after the fix: 300 X windows over
three X clients stay drawn and connected, and the server is still refused
at its budget (1250 on this container's 20000-fd table, at the 626th
window).

The entry below the line is the original filing, kept for the history. Its
guess, "the server process going away — not a scoot refusal (caps
untouched …)", was wrong. It looked only at the X-side caps, and the one
that tripped was on the Wayland side.

## What was measured

Environment: Claude Code web container (root, no GPU, no dev VM),
XWayland 24.1.13 (`/nix/store/iz5m97qfzkdv6ahnq0v673cc28sbznlp-xwayland-24.1.13`),
disk 5.4 GB free, so the original disk-full confound is gone. Soft
`RLIMIT_NOFILE` was 20000 (hard 20000). A temporary `#[ignore]` probe
(`xwayland/tests/death_probe.rs`, deleted after measuring) used the live
harness. It mapped `PROBE_BASE` 120×90 override-redirect windows
round-robin over `PROBE_CLIENTS` x11rb connections and waited for them to
draw. Then it mapped **one window at a time**, with an X round-trip
(`GetInputFocus`) before each map and a drain after it. At every step it
printed the draw list, scoot's buffer count and the server's ledger fds,
plus the Xwayland process's `VmRSS` and `/proc/<pid>/fd` count. For the
probe only, Xwayland's stderr went to a file instead of `Stdio::null()`.
Also for the probe only, `tracing` ran under `capture_logs`. Base
`40d1cf3`, uncommitted tree with only the probe added. Commands:
`SCOOT_REQUIRE_XWAYLAND=1 PROBE_CLIENTS=… PROBE_MANAGED=… PROBE_XW_ERR=…
cargo nextest run -p scoot --features xwayland --run-ignored only
death_probe --no-capture`, or `cargo test … -- --ignored --nocapture` for
the runs longer than nextest's 120 s timeout.

| run | at rest (after base) | fatal step | reason (scoot log / Xwayland stderr) |
| --- | --- | --- | --- |
| 1 X client, 240 OR base | 128 drawn (per-X-client cap refused 112), **480 buffers, 480 ledger fds** | map 257 | `Xwayland disconnected: Protocol error 1 on object wl_shm@9: wl_shm pool refused: this compositor still holds 512 file descriptors for this client … and the maximum is 512` |
| 2 X clients, 240 OR base | 240 drawn, 480/480 | map 257 (256 drawn, 512/512 just before) | same, 512 |
| 3 X clients, 240 **managed** base | 240 managed, 480 buffers, 484 fds | map 251 | same, "still holds 506" (resizes cost a few extra pools) |
| per-client bounds lifted (temporary), 24 X clients | 3000 OR drawn, 6000 buffers, 6000 fds | none: survived to 3050 | none. Xwayland RSS 287 MB (15.8 MB idle, ~90 KB per window), its own fd count 35, unchanged throughout |
| after the fix, 8 X clients, 600 OR base | 600 drawn, 1200/1200 | map 626 (625 drawn, 1250/1250) | same message, "maximum is 1250": the budget on a 20000-fd table |

Xwayland's own stderr in the fatal runs:
`XWAYLAND: wl_shm#9: error 1: wl_shm pool refused: this compositor still
holds 512 file descriptors …` then `(EE) failed to dispatch Wayland events:
Protocol error`. Its RSS and fd count were flat up to the kill, and it
exited on the protocol error.

So:

- **Count-at-rest, not burst.** One window at a time with a round-trip
  each died at exactly the same count as the batched runs.
- **Not per X client.** The count is the server's total over all its X
  clients: 1, 2 and 3 X clients all died at 2 × 256 fds.
- **Managed windows too.** Tiled X windows die the same way, a little
  earlier because resizes allocate new pixmaps.
- **The per-X-client caps do not bound it.** In the single-client run
  scoot refused 112 of 240 menus, but XWayland still allocated and
  committed buffers for all 240. Only the window manager may set
  `_XWAYLAND_ALLOW_COMMITS` (`xwl_access_property_callback` in
  `hw/xwayland/xwayland-screen.c`), and that write is private to Smithay's
  `X11Wm`. Follow-up (now resolved):
  [`xwayland-refused-windows-still-commit`](../resolved/xwayland-refused-windows-still-commit-done.md).
- **Why 2 per window:** XWayland makes each window pixmap its own
  `wl_shm` pool, destroys the pool right after `create_buffer`
  (`xwayland-shm.c`), and double-buffers each window
  (`xwayland-window-buffers.c`). Live pool objects stayed at 0 throughout.
  The fd lives on in the buffer, which is what the ledger counts.
- **Unverified, read from source:** where explicit sync is offered (the GPU
  scanout tier), XWayland 24.1.13 also imports one syncobj timeline per
  window pixmap (`xwl_glamor_dri3_syncobj_create` per `xwl_pixmap`,
  `xwayland-glamor-gbm.c`). The ordinary 128-timeline bound would then
  have killed the server near 64 windows. The server's timeline bound is
  now its total budget too. There is no GPU here to measure it on.

## The fix

- `compositor/xwayland_budget.rs`: `is_server(client)` downcasts the
  client's data to Smithay's `XWaylandClientData`, which only
  `XWayland::spawn` inserts. `bound_for(soft) = (soft / 16).clamp(512,
  4096)`. `bound()` reads the soft limit `nofile::raise` left (new
  `nofile::soft()`, one atomic load).
- `client_fds.rs`: `admit_arrival` now takes the `Client` and admits
  against `limits_for(client)`. That is the ordinary `LIMITS`, or
  `xwayland_limits(bound)` (total and timelines at the budget, pressure
  grace unchanged: under fd pressure the server past 128 is refused like
  any contributor). `Refusal::{Total, Timelines}` now carry the bound they
  refused at, so the message names the real maximum.
- `wl_buffers.rs`: `max_buffers_for(client)` feeds the same budget to the
  live-buffer count, and `dispatch.rs`'s refusal message names it.
- Sizing (in the module doc): 4096 is 2048 X windows at the measured 2
  each. That is eight X clients each drawing all 256 windows scoot's X-side
  caps allow them, and still a sixteenth of the 65536 table, far under its
  65408 pressure line. On a 1024-fd table the budget is 512, so the
  "one connection at every bound stays under the line" arithmetic in
  `fd_pressure.rs` covers the server unchanged. A larger budget there would
  let one connection reach that table's pressure line alone.
- Deliberately **not** unbounded. Past the budget the server is still
  refused, which is the only answer the protocol leaves for an
  uninitialised `wl_shm_pool`. Without a bound, one runaway X client could
  grow scoot's fd table until fd pressure started refusing every client.

## Tests

- Fail-first, live: `xwayland/tests/server_budget.rs`,
  `many_x_windows_past_one_apps_bounds_keep_the_server`. 300 menus over 3
  X clients must all draw, the server must stay connected, every X
  connection must still answer, and the premise is checked: the server's
  ledger and the buffer count are both past 512. At `40d1cf3` plus the test
  (the ledger unchanged) it failed at `x11.rs:158` with
  `ConnectionError(IoError(… Broken pipe))` mid-burst: the server was
  gone. After the fix it passes (1.38 s). The test skips loudly where the
  fd table gives the server too small a budget for 300 windows.
- Hermetic, `xwayland_budget/tests.rs` (6): the clamp arithmetic including
  `RLIM_INFINITY`; the 1024-table budget equals the ordinary bounds; the
  server alone stays far under the pressure line; the ledger admits the
  server past 512 fds and 128 timelines and refuses it at its budget with
  the budget in the message; the buffer count does the same; an ordinary
  client keeps `LIMITS` and 512 buffers.
- Updated: the `client_fds` suites' `Refusal` assertions (now carrying
  `max`), and `arrival_cost` (now through a real `Client`).

## Benchmark (the per-arrival hot path)

`client_fds::tests::arrival_cost` (ignored, prints; an ordinary client, so
it measures the one added downcast plus the soft-limit load on the path
every pool, plane and timeline takes). Debug, `--features xwayland`, 3 runs
each, 200000 arrivals per run:

| | pool/plane | timeline |
| --- | --- | --- |
| before (`40d1cf3`) | 2.798, 2.780, 2.441 µs | 2.407, 2.495, 2.308 µs |
| after | 2.405, 2.635, 2.705 µs | 2.448, 2.454, 2.191 µs |

The difference is within run-to-run noise. The added work is a `TypeId`
comparison and an atomic load, with no allocation.

## Residual

- A refused X window still costs the server its buffers, so one runaway X
  client can on its own push the server to its (now 8× larger) budget.
  See the follow-up above. (Later the same day: withholding commits was
  measured and does not help, and a refused *managed* window turned out
  to cost nothing. Only override-redirect windows remain; see the
  follow-up.)
- The GPU-tier timeline path and the 64-per-client acquire-wait bound
  (`drm_syncobj/acquire.rs`, commits waiting on unsignalled acquire
  points, which XWayland would add across all its windows) are reasoned
  from source, not measured. The acquire-wait bound is unchanged. Measure
  it on the Asahi M2 or a GPU dev VM before deciding whether the server
  needs its own bound there too. (Later the same day it got one, from the
  XWayland source: one wait per X window committing a GPU frame, scaled
  with the fd budget to 512 on the usual table. See `xwayland_budget.rs`.
  Still reasoned, not measured on a GPU.)

---

*Original filing (2026-09-27), verbatim:*

Filed 2026-09-27 from measuring
`core/xwayland-unmanaged-aggregate-cap.md` (resolved don't-build in the
same PR). While building the aggregate scene that ticket asked about (K X
clients × 128 override-redirect menus, nobody over scoot's per-client cap),
the XWayland server died instead — three times, the same place. Serves
**daily-drive** (a runaway X app's menu storm should not take down every X
app's windows) more than computer use; behind the session-owned X socket,
hence low.

## What happens

Dev VM, XWayland 24.1.13 (the `~/xw` nix store path), real X clients over
`x11rb` mapping stacked 120×90 background-pixel override-redirect windows
in batches with a settle between, scoot's live headless harness:

- Batches up to 240/256 total: every map draws promptly, server round-trips
  fine.
- The final batch (240 → 256): the server dies mid-batch. The test X
  client's next round-trip fails with EPIPE (`Broken pipe`, os error 32);
  scoot's whole `x11_unmanaged` list drains and the unmanaged cap zeroes;
  scoot itself survives cleanly (Wayland clients unaffected, `xwm` still
  attached, no panic — the orderly server-death path).
- A 512-window synchronous burst died the same way (zero maps ever
  processed: the server died mid-burst before the harness pumped).
- 2×124 = 248 total mapped, drew, timed and stayed alive end to end.

So the threshold is somewhere in (240, 256], and it is the server process
going away — not a scoot refusal (caps untouched: 128/client max, and the
cap drains rather than trips) and not the harness (the flood-phase
callback storm that kills servers in benchmarks was explicitly kept out of
these runs after it claimed one).

## Confounds to clear before trusting the count

- The dev VM's root disk sat at 99–100% through all of these runs
  (a full `/var/cargo-target` plus session scratch; ~150–250 MB free).
  XWayland's per-window backing is `/dev/shm` (tmpfs, 1.9 GB, empty —
  checked), so disk pressure *should* be irrelevant, but reproduce with a
  freer disk before trusting the exact threshold.
- RAM was free throughout (3+ GB); not OOM by the numbers, but nobody
  watched the XWayland process's own RSS across the storm.
- Whether it is count-at-rest (~250 live OR windows) or burst rate was not
  separated: the batched runs still map 16 per batch with one settle
  between. Map 240, pause, then add one window at a time with a round-trip
  check each to pin the fatal map.

## What a fix would need to decide

Whether the death is an XWayland bug worth carrying a fork workaround for
(`docs/forks.md` — the project already carries a scoot-sh Smithay fork,
but XWayland itself is nixpkgs-pinned, a different kind of carry), a
resource limit worth documenting, or something scoot's spawn should harden
against (restart? refuse maps past a scoot-side total?). Do not implement
here.
