---
title: "Withhold frame callbacks from layer surfaces nobody can see — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Withhold frame callbacks from layer surfaces nobody can see

**RESOLVED (2026-09-27) in 732aaf1, PR #279.** The frame-callback pass in `State::render` skips a
layer surface that already committed a buffer and is fully covered by
opaque window content on its output; everything else is served exactly as
before. No user-facing surface: no config, binds, CLI or IPC change, so no
README update (stated explicitly rather than skipped silently).

## Verdict record

### What landed

- `compositor/layer_shell/occlusion.rs` (new): `withhold_frame` plus three
  helpers. Overlay is always served (above every window); top is withheld
  only under a covering fullscreen window (which hides the whole layer --
  the same `above_windows` answer rendering, hit-testing and focus use);
  bottom/background are withheld under any opaque window spanning the
  output. Opacity is Smithay's own `RendererSurfaceState::opaque_regions`
  (an opaque-format buffer already folds to a whole-surface region there),
  single-region containment against the output rect in `i64` (`i32` on
  client-influenced coordinates could wrap near `i32::MAX` and fake
  coverage -- found in bug-bash, hardened before review), the root's own
  `wp_alpha_modifier_v1` multiplier required at exactly 1.0, X11
  `_NET_WM_WINDOW_OPACITY` honoured the way the render stack applies it. No
  heap allocation on the path (state reads, integer arithmetic over the
  already-computed regions; `wl_surface()` is a borrow or a refcount bump).
  The review round added the placement filter the first version missed: only
  a window placed on the output (`output_clip::placed_on`, the same stamp a
  frame draws by) is a spanning-cover candidate, so a window on the
  neighbour whose opaque region contains this output's frame withholds
  nothing here. The fullscreen path needed no change (already per-output via
  the core).
- `headless.rs`: the layer loop calls it; a skipped callback stays queued
  server-side and the next served frame completes it, so none is ever lost.
- Tests: `fullscreen/tests/occlusion.rs` (7 tests, one per hard
  constraint plus the layer-depth rule; the review round added an 8th, a
  cross-output spanning cover that must withhold nothing where it is not
  placed, plus the `DrawXrgbSized`/`CreateLayerOn` fixture steps it needs),
  and plain-protocol fixture steps
  (`DrawXrgb`, `SetOpaque`, `Wallpaper`) ungated from `gpu-scanout` so they
  run on the default build, plus `CreateLayerDeferred`/`DrawLayer` (the
  first-attach split) and `RequestLayerFrame`/`ReportLayerFrames`.

### Deliberate misses (conservative = serve, documented in the module)

Multi-rect unions, subsurface-composed opacity, layer-covers-layer, and any
cover the compositor cannot see yet (unmapped window, unpaired X window). An
alpha-modified or translucent cover is always served. YUV is *not* a miss:
Smithay folds a YUV buffer (its YUV fourccs carry no alpha --
`format::has_alpha` is false for them, and `buffer_has_alpha` therefore
reports `Some(false)`) into a whole-surface opaque region, so a YUV cover
withholds, read from the same `opaque_regions` the damage tracker uses. The
review round corrected the module doc, which had claimed the opposite, but
added no YUV test: no live YUV cover can be built in-harness -- the
compositor advertises only `Argb8888`/`Xrgb8888` shm, Smithay refuses a
`Yuyv` pool (no bytes-per-pixel entry) and an unadvertised `Nv12` pool, and
pixman imports neither -- YUV arrives only via dmabuf, which the harness has
no path for. There is nothing of ours to pin: our code only reads Smithay's
regions.

### Bug-bash

Lock audit: the check runs under the output's layer-map guard and takes
surface-state locks (including window roots, a wider set than the
pre-existing `send_frame`-under-guard). No `with_states` closure anywhere
in the codebase takes the layer-map guard (checked mechanically), and this
path never re-takes it, so no new ordering and no same-output double-take.
Overflow audit above. Teardown paths keep the old unconditional behaviour
(dead surface: served, i.e. `send_frame` no-op as before).

### Evidence (dev VM, 2026-09-27, commit 732aaf1; base is main 3594c98)

- Constraint tests: 7/7 pass on 732aaf1. Mutation check (call site
  stashed): 6 fail pre-fix with `[1, 1]` (every callback served), the
  tiled-served control passes both ways.
- Live A/B, release binaries, `--headless` 1280x800 pixman, scratch
  frame-paced shm-gradient wallpaper + static `Xrgb8888` fullscreen cover
  (both in `/tmp`, not the repo), jiffies over fixed windows, two rounds
  each. Client CPU covered: base 111j+108j/15s at 52fps both rounds, head
  0j+0j at 0fps both rounds. Compositor CPU covered: base 59j+60j, head
  0j+0j. Uncovered parity (no behaviour change when visible): client
  103/101 vs 107/104, scoot 97/93 vs 101/101, ~48-52fps both. Resume:
  full rate back on cover kill (54/54fps head, 48/48 base).
- Compositor frame time: debug `render_frame_cost` BEST-of-5x500, 800x800
  pixman -- base empty 72.634µs vs head 60.752µs, 8-window 258.506 vs
  233.524, multi empty 68.635/127.977 vs 56.189/115.055, multi 8-window
  257.04/328.446 vs 229.511/271.114 (head within noise, favourable
  throughout; these scenes map no layer surfaces, so the new code is
  unreachable there by construction). The release bench could not link on
  this VM (fat-LTO rustc OOM-killed at 3.7GB of 3.9GB, no swap) -- recorded
  so the reviewer does not re-try blindly.
- Layered frame time (review round, dev VM, 2026-09-27, debug 200x200
  pixman): a scratch BEST-of-5x500 scene with 3 mapped windows plus a
  wallpaper and a bar, so the occlusion scan runs on every frame (the scenes
  above map no layers and never reach it). Base (filter reverted in-tree)
  BEST 323.364/343.409µs vs head 293.086/285.775/296.945µs per frame. Head
  at or below base in every sample -- the gap is shared-host noise, not a
  speedup (the filter adds one user-data read per candidate window; it
  cannot make a frame faster), and either way there is no per-frame cost
  problem, so the per-output cover result stays computed per layer rather
  than hoisted. The scratch bench was deleted after measuring; the numbers
  live here.
- Full workspace nextest: 2178/2179, `clippy -p scoot --all-targets
  -- -D warnings` clean, `fmt --check` clean, `smoke-test.sh` rc=0, and
  `cargo check --all-targets` clean under `--features xwayland` and
  `--features gpu-scanout` (the ungated fixture steps compile there too).
  The one failure is `scootbg ... a_live_socket_with_a_full_backlog...`,
  a pure-socket test in an untouched crate (no `crates/scootbg` file in
  this diff; fails identically in isolation) -- pre-existing/environmental,
  left for its lane owner.
- Environment notes: the dev VM's disk was 100% full (148K free), which
  broke builds mid-session; freed with `sudo journalctl
  --vacuum-size=100M` (761M; system logs only, no agent files touched).
  Smithay claims verified against the pinned fork rev `74edbf3`
  (`crates/scoot/Cargo.toml`), not the older rev `CLAUDE.md` still names.

## Original entry

Found in review of the scootbg plan (PR #264, 2026-09-26).

After each rendered frame, scoot sends `wl_surface.frame` to every mapped
layer surface on the output (the post-render loop in
`crates/scoot/src/compositor/headless.rs`, "Sent to every mapped layer
surface on this output"). There is no occlusion check. A client that
paces animation by frame callbacks, which is the protocol's intended way
to stop drawing when unseen, therefore never learns it is covered: an
animated wallpaper under a fullscreen video keeps decoding and blending
at the video's frame rate, on the CPU under pixman.

This matters for scootbg's animated wallpapers
([`docs/scootbg/backlog/animated-images.md`](../../scootbg/backlog/animated-images.md))
and for any other animated background or bottom-layer client.

## Constraints on a fix

- The current behaviour is deliberate for a reason that still holds: a
  client may ask for a callback before its first attach, and withholding
  it would stall the frame that unsticks it. So only a surface that has
  already committed a buffer, and is fully covered, is a candidate.
- "Fully covered" means covered by opaque content on that output (a
  fullscreen window with an opaque region or opaque format, or an
  opaque-format window spanning the output), computed from what the frame
  already knows, not with a per-frame allocation.
- The callback resumes on the first frame where any of the surface is
  visible again, and a withheld callback is never lost (the client gets
  the next one).
- Measure: CPU of an animated background client under a fullscreen opaque
  window, before and after, plus a check that the compositor's own frame
  time does not grow.
