---
title: "Animated wallpapers: GIF, APNG, animated WebP (milestone 2)"
status: "open"
area: "scootbg"
priority: "medium"
blocked: "reuses the frame pacing built for transitions.md"
---

# Animated wallpapers: GIF, APNG, animated WebP (milestone 2)

- Decode frames once, scale once per output; keep them compressed (or as
  diffs) under a memory cap, and refuse or downscale animations past it.
- Pace with frame callbacks. That only makes a covered wallpaper free if
  the compositor withholds callbacks from surfaces nobody can see, and
  **scoot does not today**: it sends `frame` to every mapped layer surface
  on each rendered frame, with no occlusion check (`headless.rs`, the
  post-render path). A fullscreen video over an animated wallpaper would
  keep scootbg blending at the video's frame rate. So this item needs
  [the scoot-side fix](../../backlog/core/frame-callbacks-for-hidden-surfaces.md)
  first, or its own pause (e.g. `ext-foreign-toplevel` fullscreen state),
  measured either way.
- `--no-animate` to show the first frame only.

## Stage 1 (landed): stills with caps, no playback

GIF decodes (first frame), animated GIF/APNG/animated WebP show their
first frame as a still with zero wakeups. With animation checks on (the
default, `animate` absent/`"true"` on the wire) the full decode runs
first and past 64 frames / 64 MiB of frames the `set` is refused naming
`--no-animate`; `--no-animate` (`animate:false`) decodes the static way
with no animation caps. `gif` 0.14.2 (`default-features = false`) is the
only new decoder; APNG/WebP reuse the in-tree ones. Help, protocol,
`Image.animate` and site docs all say first-frame-only; nothing claims
playback.

Note: the scoot-side occlusion fix above has since landed
(`frame-callbacks-for-hidden-surfaces-done.md`), so covered outputs cost
nothing once playback paces off frame callbacks.

## Playback remainder (precise; not started)

Drive frames on the daemon: `State.animations` + a poll-loop slot paced
off the poll timeout (no timerfd: the `RLIMIT_NOFILE` exhaustion tests
in `crates/scootbg/tests/` pin the daemon's fd table — reuse the
transitions machinery in `daemon/transition.rs`, which holds its own
timerfd while transitioning, only as a pacing reference),
`RoundTrip::AnimFrame` + `wl_callback`/presentation-feedback dispatch in
`daemon/surfaces.rs`, worker `Done` carrying per-target frame buffers +
delays (per-output scaled buffers, no cross-output sharing; freed when
the animation stops), the `reconcile` hook order (transition →
animation → static) in `daemon/change.rs`, the `section.rs` `animate`
key (strict; fingerprint moves with it), `query`'s per-output `animated`
bool, loop counts (loop-forever is the conservative default; document
it), transition-into-animated (first frame transitions, then plays).
Rotation has since merged (PR #514, `feat/scootbg: rotate through a
directory on one timer`), so the `--every` interaction is decided, not
moot — stage 1 behavior, kept by the rebase:
- Slideshow steps show each animated file's first frame, checked per
  step like one image's `set` (`drive_due` and the first-file trial
  both run with the animation checks on): past 64 frames / 64 MiB that
  step fails like any undecodable file, until the next step.
- `--every` with `--no-animate` is refused on the CLI
  (`NoAnimateWithEvery`, exit 2) and on the wire
  (`AnimateWithSlideshow`): stilling is per image, and a slideshow steps
  through many.
- `--no-animate` stills are live-only: the state file (still format v3)
  never persists `animate`, so after a restart the caps are checked
  again and an over-cap animation is refused then.
- The GIF/APNG RGBA canvases allocate through the checked `buffer()`,
  so exhaustion is a clean `OutOfMemory` refusal (pinned by
  `tests/animated_oom.rs`), like every other pixel buffer.
Stage 2 re-decides the slideshow half: whether steps play or still,
threading `animate` through `SlideshowRequest`/rotation/steps and the
state file, per-step loop counts, and transition-into-animated-step.
Tests for each: disposal/blend pixel fixtures, pacing math, cap refusal,
zero-wakeup-when-static/covered end-to-end on headless scoot,
replace-mid-animation, DPMS/output-removed mid-animation,
frame-callback-never-arriving (no spin), plus the ratchet (release
size/`.text`, RSS/PSS/wakeups, CPU per frame) and the full verification
set (nextest with `SCOOTBG_REQUIRE_SCOOT=1`, clippy `-D warnings`, fmt,
`cargo deny check`, `scripts/backlog check`, `nix build .#docs-site`,
flake-loop timing tests 20x, CI green).
