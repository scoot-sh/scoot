---
title: "single-pixel-buffer-v1: solid-color 1x1 buffers with no shm — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `single-pixel-buffer-v1` — DONE

~~A trivial protocol for a client to get a solid-color 1x1 buffer without
allocating a real one; some toolkits use it for cheap fills~~ — DONE, one of
the bundle children of
`docs/backlog/protocols/protocol-gaps-general.md` (which now marks it done
the same way). The other two remainders there (`presentation-time`,
`relative-pointer`) are separate designs and stay open.

## What landed

`wp_single_pixel_buffer_manager_v1` (version 1), advertised to every client
with no filter. Unlike the three hand-rolled predecessors (`gamma_control`,
`output_management`, `wlr-foreign-toplevel-management`), there was nothing
to hand-roll: the pinned Smithay rev (`0ff0098`, verified in source rather
than assumed) already carries the whole protocol under
`src/wayland/single_pixel_buffer/` — a `SinglePixelBufferState` global plus
`get_single_pixel_buffer`, 1x1 dimensions through `buffer_dimensions`, an
import skip in `update_surface`, and a `SolidColor` render element the
pixman backend draws with `draw_solid`. So flexwm's side is three lines in
`state.rs` (import, field, construction — the same hold-only-to-keep-the
global-alive shape as `cursor_shape_manager_state`), a `single_pixel_buffer.rs`
module doc recording the spec-derived edge cases, and five real-client tests.
No Smithay patch vendored; everything stays in flexwm's handler layer, which
here means the state that keeps the global alive.

## Edge cases (all spec-derived, all pinned by tests)

- **Malformed colors are not expressible.** Every channel's valid range is
  the full `uint` (`0` to `u32::MAX`, read as a percentage), so there is no
  out-of-range value to clamp or refuse. The boundaries are pinned instead:
  `0` and `u32::MAX` store exactly, `rgba8888` maps `0x80808080` to exactly
  128, `rgba32f` maps the endpoints to exactly `0.0`/`1.0`, and `has_alpha`
  answers both ways.
- **Manager `destroy` while buffers live.** The spec says the children are
  unaffected; the test destroys the manager first and still maps and renders
  from the orphaned buffer.
- **Buffer `destroy` while attached.** Legal per Wayland; asserted as
  survival (client still connected, compositor still renders a full frame),
  not pixels, since what a destroyed buffer draws is Smithay's call.
- **shm-pool accounting.** These buffers allocate no pool, so
  `dispatch.rs`'s guards (all `TypeId`-gated to `wl_shm`/`wl_shm_pool`) never
  see them — nothing claimed against the 128-live-pool budget, and no bypass
  of a limit that should apply, since there is no fd, mapping or reservation
  to bound. Pinned by a pool-count assertion after creating three buffers.
- **dmabuf feedback / screencopy.** No interaction by construction: no dmabuf
  object is created, named or imported anywhere on this path.

## Evidence

- Fail-first, dev VM (`ssh -p 2222 dev@localhost`, branch
  `feat/single-pixel-buffer-v1`, pre-fix tree): all five new tests fail with
  the global missing — `the_manager_global_is_advertised` panics on
  `wp_single_pixel_buffer_manager_v1 was not advertised`, the other four on
  `no wp_single_pixel_buffer_manager_v1 -- the global is missing`.
- Post-fix, same VM: `cargo test -p flexwm` 819 + 3 pass (5 new),
  `cargo nextest run --workspace` 924 pass 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean (Mac-side),
  `scripts/smoke-test.sh` (`SMOKE_PREFIX=/tmp/smoke-spb`, `--headless`)
  exit 0 with zero `BUG` lines.
- Live, same VM: `wayland-info` against a headless flexwm lists
  `wp_single_pixel_buffer_manager_v1, version 1`; real `foot` binds it
  (`wl_registry.bind(17, "wp_single_pixel_buffer_manager_v1", 1, ...)`) but
  mints no buffer in that run — create/attach/render is proven by the
  harness tests instead, which drive a real client through viewport-scaled
  attach, commit and a pixman read-back.
- Benchmark: none. Stated cost shape: bind-time plus one four-`u32`
  allocation per buffer; nothing runs per frame or per event.

## What this deliberately leaves open

Nothing on this protocol. Its bundle neighbours (`presentation-time`,
`relative-pointer`) are untouched and stay in
`docs/backlog/protocols/protocol-gaps-general.md`.
