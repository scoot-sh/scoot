---
title: "Rounded window corners"
status: "open"
area: "rendering"
priority: "low"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block, though milestone 6 changes what this costs"
---

# Rounded window corners

Requested 2026-09-19. scoot already draws its own decorations (gap, focus
ring, background colour — `decorations.rs`, `[appearance]`), so this is a
compositor-side effect, not something delegated to clients. The natural
surface is `[appearance] corner_radius` alongside the settings that exist.

`CLAUDE.md`'s priority order applies directly and should be read before
starting: *beauty comes second, and if a visual feature costs meaningful
performance, correctness risk or coupling, the engineering bar wins and
polish waits or gets a cheaper implementation.* This entry exists to record
what that cost actually is, so the decision is made on numbers rather than
on taste.

## The cost is not the corners, it's the opacity

Drawing four rounded corners is cheap. What is not cheap is what rounding
does to the **opaque region**: a rounded window no longer covers its own
rectangle, so the compositor can no longer skip what is behind it. Every
window below has to be composited where it was previously occluded, on a
CPU rasteriser, every frame that region is damaged.

That is the number that decides this. It is a per-frame cost proportional to
overlap, not to radius, and it is invisible in a single-window benchmark —
so **measure it with stacked, overlapping windows**, which is precisely the
case the layout makes common.

Milestone 6 changes the calculus and is why this is sequenced after it. On
GLES an alpha mask is close to free; on pixman it is not. A reasonable
outcome is that this ships **renderer-aware** — rounded on the GPU tier,
square or opt-in on pixman — rather than unconditionally. Deciding that
before the GPU tier settles would be guessing.

## Things that have to follow the radius, or it will look wrong

- **The focus ring.** Drawn as a border today; a square ring around a
  rounded window is worse than no rounding at all.
- **Damage tracking.** The damaged region for a rounded window is not its
  bounding box. Getting this wrong shows up as corner artifacts that persist
  until something else forces a repaint — the classic symptom.
- **Capture.** `screenshot` and `screencopy` read the composited frame, so
  rounding appears in captures. That is probably correct, but an agent
  diffing screenshots against expected pixels will see it, and the
  agent-facing docs should say so.
- **Subsurfaces and popups.** A popup is its own surface; whether menus
  round too is a decision, not a default.

## Prior art worth reading before choosing an approach

niri and Hyprland both do this on a GPU renderer with a shader; neither has
scoot's constraint of a CPU rasteriser being the *default* path. Their
approach is not directly portable, and the licence rule in `CLAUDE.md`
applies to niri's code (GPL-3.0) regardless.
