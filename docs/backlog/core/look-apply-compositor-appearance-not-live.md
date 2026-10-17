---
title: "look apply reports reloaded compositor but decorations keep the old look until restart"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# look apply reports reloaded compositor but decorations keep the old look until restart

Filed 2026-10-17 from the webtop field test (T-webtop). Serves **daily-drive**:
`look apply` is documented to reload the compositor live; it does not.

## The gap

Session started with `ginger-night` current (focus `#FF9A30`). Applied
`moonrise`, `music-desk` (focus `#3D579A`), `radial-burst`, `vinyl-sunset` in
turn, then `moonrise` and `music-desk` again — every apply answered
`applied` + `reloaded: compositor, bar wayland-1`, generations advanced to 15.
Pixel-sampling the `scoot msg screenshot` PNGs (tolerance 24 per channel,
full-frame scan): 465 px of ginger `#FF9A30`, ZERO px of music-desk
`#3D579A` — in frames taken up to 10 minutes after the apply, and in a window
spawned fresh AFTER the apply (so it is not a per-window map-time cache).
Screenshotting immediately after an apply returns the pre-reload frame each
time (five identical md5s); that half is timing, not a bug.

Restart the session with `music-desk` current: the fresh session renders
`#3D579A` (549 px, zero ginger). Apply `ginger-night` live, wait 8 s,
screenshot: byte-identical to the pre-apply frame, still `#3D579A`.
So the look layer applies at startup and never again for compositor
decorations, while the reply claims a live reload.

Environment: `scoot 0.3.0` release binary, `--headless --outputs 1`,
cpu renderer, aarch64, in `lscr.io/linuxserver/webtop:debian-xfce`
(stand-in: `--nested` cannot start there — see `nested-steals-host-wayland-socket`).
Screenshots: `scratchpad/webtop-shots/` in the T-webtop task dir.

## What to do

Find where the compositor's look-layer reload drops appearance on the floor:
suspects are the reload applying the link for state/report but the render
path keeping the startup snapshot, or appearance deliberately excluded from
the live path while the reply still names the compositor. Either fix the
reload or narrow the reply + docs (`looks/index.md` promises "appearance and
wallpaper swap", `apply` promises per-app reload "in order"); a reply that
names a component must have reloaded it.

Pin the edge cases: which appearance keys (if any) DO swap live today
(wallpaper? bar claims its own reload — verify visually, not just by reply);
whether a restart-only subset is acceptable and documented (matches the
`[virtual_input] binds`-style restart-only precedent) or the whole layer must
go live; the `next/prev/random/--for` paths ride the same transaction (verify
they inherit the fix — the field test exercised all of them).

## Not in this ticket

The nested socket-steal, the foot `initial-color-theme` refusal, and the
webtop glibc-tag docs (separate tickets from the same test). The bar's visual
reload was not verified (clock-only bar in the shots) — check while here.
