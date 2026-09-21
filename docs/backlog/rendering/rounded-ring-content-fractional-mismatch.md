---
title: "corner_radius: ring and window content corners do not line up at fractional output scale"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# corner_radius: ring and window content corners do not line up at fractional output scale

Filed as gh issue #205 (2026-09-21). With `corner_radius` set, the focus
ring curves but the corner it curves around does not match the window
content behind it — each corner shows a visible mismatch rather than the
"ring and content rounded together" `docs/configuration.md` describes.

Repro from the issue (scoot from the flake, pixman renderer, `--tty` on
virtio-gpu in a vfkit VM on an Apple Silicon host; `foot` with a
translucent background so the corners are visible rather than guessed at):

```toml
[appearance]
corner_radius = 10
focus_ring_width = 4
cursor_theme = "Adwaita"
cursor_size = 24

[output]
scale = 1.5
```

Display at the time: 2952x1660 device pixels (1968x1107 logical at 1.5),
no `--mode`.

## Suspected mechanism (reporter's hypothesis, not yet confirmed)

`corner_radius` is documented in *logical* pixels and the session runs at
a fractional scale (1.5). If the ring geometry and the content rounding
arrive at their device-pixel radius by different routes — one scaling the
logical radius, one rounding to whole device pixels, or one using
`ceil(scale)` = 2 where the other uses 1.5 — they disagree by a pixel or
two exactly where it shows, growing with the radius. Concrete suspects in
tree: `rounded::physical_radius` (`configured * scale`, rounded,
`crates/scoot/src/compositor/rounded.rs`) vs `rounded::ring_layout`'s
canvas rounding (offset-then-round minus origin-round, matching Smithay's
memory-render element) vs the client's own fractional buffer scale
(`output_scale.rs`, `wp_fractional_scale_v1` + integer companion).

Separation test from the issue: same `corner_radius` at `scale = 1.0` and
`scale = 2.0` vs `1.5` — if integer scales line up and only 1.5 is wrong,
the rounding-route split is confirmed.

## Prior art (resolved, not this bug)

- `resolved/rounded-window-corners-done.md` — shipped `corner_radius`;
  ring and clip share `clip_rect`/`cut_width` by construction, pixel suite
  passes on pixman and GLES. Suite runs at scale 1.0; no fractional-scale
  pixel pin exists.
- `resolved/ring-hole-fractional-drift-done.md` — fractional drift in the
  painted-ring origin refresh (reuse check now compares the plan too).
  Same file family, different symptom (origin, not radius).
- `resolved/output-scaling-done.md`,
  `resolved/fractional-scale-integer-companion-done.md`,
  `resolved/ghostty-fails-at-1-5-done.md` — the bind-time scale
  advertisement the clients see; relevant if the content side turns out to
  be the client's rounding rather than the compositor's.

## Carried notes from the issue (both arguably intended, kept here)

- Radius applies to every window equally; the per-window clamp to half the
  smaller dimension turns tiny windows into stadiums. Documented behavior.
- Popups staying square next to rounded parents is a documented open
  decision (`rounded-window-corners-done.md` follow-ons); noticeable when a
  launcher popup sits over a rounded window, but not this bug.

## What done looks like

- Fail-first pixel/harness pin at scale 1.5 (ring inner edge vs content
  outer edge coincide; integer scales stay green), then the rounding fix.
- Reporter offered to test a fix and to run the 1.0/2.0 comparison —
  take them up on it before closing.
