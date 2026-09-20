---
title: "Every host configure is acted on immediately, so a drag mints a mode per pixel step — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Every host configure is acted on immediately, so a drag mints a mode per pixel step — RESOLVED.

## The entry as filed

Found by review of PR #151, confirmed live: `wlr-randr` on a nested
session reports 2 modes after the first configure and 4 after three
resizes, including one size that never rendered. A host resize is not one
event — dragging a webtop browser window 800px→1900px sends a configure
per pixel step, and scoot acted on every one. Each distinct size appended
to `Output::modes` (nothing pruned it), minted a `zwlr_output_mode_v1`
per output-management client, and slowed the next refresh (linear scan) —
O(N²) over a drag, ~1100 modes and ~1100 protocol objects per client.

The ticket named the shape (coalesce configures to the next render tick),
rejected `delete_mode`-on-the-nested-path as fixing only the list while
leaving a full pool rebuild per pixel, and filed a watch-for: `set_mode`
marks the new mode preferred before a failed resize restores the old one,
so a client bound across a failed resize can see two preferred flags, and
the failed mode stays in `Output::modes` for later binds.

## Resolution (2026-09-20) — coalesced as filed, watch-for confirmed and fixed

**Coalescing: queue in dispatch, drain in render.** A `Resize`
classification in `nested_dispatch.rs` now overwrites a two-`i32` slot
(`PendingResize` on `Host` — no allocation at configure rate, latest wins)
and calls `request_render`, instead of rebuilding the pool and the render
target synchronously. `State::render` drains at most one queued size per
frame tick, ahead of the clean-screen early return, and the frame
composited in that same tick is already at the new size
(`Host::drain_pending_resize`, `Host::queue_resize` in `nested.rs`).
`FirstConfigure` stays immediate (nothing can render before it exists);
same-size configures stay `Nothing` before they can queue. A drag that
comes back to the starting size within one frame drains to nothing
(`take_if_changed`), pinned by unit test.

**Watch-for: confirmed real, fixed on the failure path.** Re-derived
against the pinned Smithay rev (`0ff00983`): `set_preferred` and
`change_current_state` each push unknown modes into `Output::modes`, so a
failed `resize_output` left the never-rendered size in the list, and the
*next* successful `refresh_output_heads` announced it as a brand-new
`zwlr_output_mode_v1` next to the real one — proven fail-first (the
failing assertion showed `ModeSize(1, 2147483647, 1)` announced alongside
the real mode). The failure path now `delete_mode`s the failed size after
restoring the old one (guarded on differing — a same-size call names the
still-current mode, which `delete_mode` would clear). Later binds never
learn the failed size; output-management never mints it.

**One residual is inherent, not deferred.** An already-bound `wl_output`
client keeps the transient: the failed size's `Current|Preferred`
announcement is sent synchronously before the build fails, and `wl_output`
has no un-prefer and no mode withdrawal (`delete_mode`'s own doc says
existing clients are not de-advertised). Verified in source
(`src/wayland/output/handlers.rs` bind sends every known mode;
`src/wayland/output/mod.rs` sends synchronously). It opens only on a
resize that fails (a pool that would not allocate), and the recovery —
old size current and preferred again — is pinned by test. Stated here so
it is not re-filed as a discovery.

## Evidence

All captured on the dev VM (`ssh -p 2222 dev@localhost`), tree at branch
`backlog/coalesce-host-configures` (base `fb7ff53`), Smithay pin
`0ff00983`.

- `cargo nextest run --workspace`: **1147 passed, 4 skipped, 0 failed**.
- `cargo test -p scoot --bin scoot` (the fallback runner): **1042
  passed, 0 failed, 4 ignored** — no cross-test-interference shape.
- `cargo clippy -p scoot --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p scoot`: clean.
- `scripts/smoke-test.sh` (default `--headless`): exit 0.
- `MODE=--nested` smoke under an outer-scoot host: exit 0.
- Full `scripts/nested-resize-bugbash.sh` (release binary): client-mapped
  resize follows (window 273→372 as output 582→780), 12 back-to-back
  resizes keep the session alive at the right size, screenshot 780x776 at
  the final size, same-size churn leaves the mode count 4→4, host death
  exits the inner — the last proven pre-existing (identical pre-fix),
  not a regression.
- Fail-first (fix temporarily commented out, `delete_mode` line only):
  exactly the three predicted tests fail —
  `a_failed_resize_leaves_no_mode_behind`,
  `a_client_bound_after_a_failed_resize_never_sees_its_size`,
  `a_failed_resize_leaves_no_mode_for_the_next_refresh`
  (showing the `2147483647x1` announcement); the two recovery pins pass
  throughout. Fix restored afterwards (`git diff` confirms the line).
- Live flood, outer scoot as host with 80 distinct `[layout]
  column_widths`, inner `--nested`, `wlr-randr` mode counts:
  - pre-fix (debug): 79 rapid configures in 147 ms → **2 → 81 modes**
    (one per configure — the bug, reproduced).
  - post-fix (debug): 76 rapid configures → **30 modes**.
  - post-fix (release): 79 rapid configures in 447 ms → **25 modes**
    (≈ one drain per 16 ms tick elapsed; the bound is structural).
  - Settled control, both builds: every resize followed, sizes track
    outer exactly (240/247/254…; final inner 226x776 == outer 226x776).
- Per-resize cost, `headless/bench.rs` ignored benches (release, dev VM):
  - `resize_cost` pixman: **BEST 26.0 µs** per resize (renderer half).
  - `resize_cost` gles (`SCOOT_TEST_RENDERER=gles`): **BEST 15.5 ms**
    per resize — the ticket's 16.6 ms claim, confirmed: a whole 60 Hz
    frame per rebuild, which is what makes one-per-tick (vs
    one-per-pixel-step) load-bearing under gles.
  - `render_frame_cost` pixman post-fix: **60.8 µs** empty / **98.9 µs**
    8-window per frame — the drain is one `None`-check on this path
    (no host in the harness), i.e. noise inside those numbers.
- Pool-rebuild-inclusive nested total, release, outer action → inner
  `msg outputs` change (5 ms poll granularity included):
  - pre-fix: 13–20 ms, median ~18 ms.
  - post-fix: 29–37 ms, median ~31 ms.
  - Delta ≈ +13 ms ≈ one frame-tick wait: the stated price of
    coalescing on a settled resize. Against the 16.6 ms budget: one
    settled resize costs ~2 frames end to end (tick wait + two IPC
    round trips + pool + target + frame), while a drag costs ~1
    rebuild per frame instead of ~1 per pixel step.

## Deliberately left out (per the ticket's scope)

- **In-place GLES target resize** (`docs/tty.md`'s deeper fix): a gles
  resize still rebuilds the whole EGL context + shaders (15.5 ms); this
  PR only makes it happen once per frame. Stated, not hidden.
- **The unbounded product**: 8192×32767 still passes both axis guards
  (untouched — `git diff` shows no guard change), so ~2 GB pool + ~1 GB
  target is still admittable per configure. Coalescing shrinks the
  window from per-pixel-step to per-frame; it does not bound the
  product. Honest, as the ticket asked.
- **Multi-output** (`docs/backlog/core/multi-output.md`), **axis-guard
  changes**, `screencopy-dmabuf-capture.md`: untouched.
- **No README change**: internal perf fix, no new/changed config,
  keybinding, CLI flag, or IPC surface. Settled-resize latency grows by
  ≤ one frame (measured +13 ms); that is a cost of the fix, documented
  here, not a user-facing feature.
- `headless/bench.rs` still measures the renderer half only (its doc
  says so); the pool-rebuild-inclusive numbers live in this record,
  not in an ignored bench that cannot build a host pool without a
  live compositor.
