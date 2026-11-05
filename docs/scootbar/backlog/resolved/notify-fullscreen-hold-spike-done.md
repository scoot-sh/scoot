---
title: "Spike: hold non-critical cards under a fullscreen window via a `top` layer surface"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-11-05"
---

# Spike: a fullscreen hold for scootnotify

Filed 2026-10-12 in the #579 fix round. It replaces the first draft's foreign-toplevel
machinery with a cheaper idea to measure. Serves **daily-drive**. Status research; v1 shows
cards over fullscreen windows, as mako does today.

## The idea

scoot withholds frame callbacks from a `top` layer surface that has committed a buffer, while an
opaque fullscreen window covers its output (`crates/scoot/src/compositor/layer_shell/occlusion.rs:15-26,96-113`).
scootnotify's expiry clock starts at the first frame callback, so non-critical cards on a `top`
surface would be held for free. Critical cards stay on the `overlay` surface.

## What to measure

- a translucent fullscreen window, where the callback is served and the clock starts while the
  card is hidden;
- the cost of a second surface;
- behavior on sway (the rule is scoot-specific).

## Acceptance

A recorded go or no-go in this entry, and the design updated.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/configure.md` with the fullscreen-hold behavior and its option, if the spike ships one; else a dev/ note only. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Resolution (2026-11-05)

**Verdict: conditional GO.** The mechanism works and is cheap, but it cannot ship against scoot
v0.7.9. A one-line core change comes first
([`top-layer-withhold-under-any-fullscreen`](../../../backlog/resolved/top-layer-withhold-under-any-fullscreen-done.md)),
and the card stack has to be split across two surfaces. Nothing shipped in this PR: it is a
finding, with the design updated (`dev/design/scootnotify.md`, Fullscreen) and no user-visible
change, so `site/` is untouched.

### Method

Throwaway Wayland client `fsprobe` (wayland-client 0.31 and the wlr layer-shell bindings, built
outside the repo on the dev VM; source in the PR description). It opens a fullscreen xdg toplevel
in one of four buffer shapes (`opaque` XRGB, `argb-region` ARGB with a full opaque region,
`argb-plain` ARGB alpha 255 and no region, `translucent` ARGB alpha 128), and 300x100 layer
surfaces on `top` and `overlay` that log the time of each `wl_surface.frame` callback after commit,
the way scootnotify starts an expiry clock. Compositor: the **v0.7.9 release binary**
(`gh release download v0.7.9`, aarch64, sha256 `eb7b5faa3dcf6f4a5047f80a8c08722b169e9c315f8cf9965420f96a4d18e3ea`;
`scootnotify` `3f87379c65af7fbe4d14765e5e08b965fa10bf039209fe6320b06b9af5b41aa8`), run as
`scoot --headless --width 1200 --height 800 --renderer cpu` on the dev VM (aarch64). NixOS cannot
exec it, so it ran through `ld-linux-aarch64.so.1` of glibc 2.44 with `--library-path` over the
store paths of libinput, libseat, libudev, pixman and libxkbcommon (no patchelf on the VM). Times
are `Instant` milliseconds since the client started; the repo was at `f17deed97`.

### 1. Opaque fullscreen window: held

| Window | `top` first callback | `overlay` first callback |
|---|---|---|
| none (baseline) | 18.7 ms | 18.7 ms |
| `opaque` (XRGB) | **never** (0 callbacks in 4 s) | 18.3 ms |
| `argb-region` (ARGB, full opaque region) | **never** | 18.6 ms |
| real `foot` 1.28 fullscreen, `[colors-dark] alpha=1.0` | **never** | 19.4 ms |

The `top` surface is absent from the screenshot (`msg screenshot`): only the `overlay` card draws
(`opaque` window in green, cards in blue). Un-fullscreening at +2535 ms (`msg action
toggle-fullscreen`) served the withheld callback at +2562.9 ms (**27 ms later**), and the `top`
card reappeared. So a card held under a video starts its clock when the video ends, as designed.

### 2. Not opaque: the clock starts while the card is hidden

| Window | `top` first callback | `top` visible? |
|---|---|---|
| `argb-plain` (ARGB alpha 255, no opaque region) | 18.7 ms, **served** | no |
| `translucent` (ARGB alpha 128) | 18.8 ms, **served** | **no** |

In the `translucent` screenshot the `top` card (y 10) is absent and the `overlay` card (y 120)
shows: scoot hides every `top` surface under any covering fullscreen window
(`layer_shell::above_windows`, `render/elements.rs:433`), but withholds its callbacks only when
the cover is also opaque (`occlusion.rs`, the `Layer::Top` arm). The two disagree, so a card
expires unseen. This is a scoot defect, not a limit of the idea: the opacity guard belongs to
`bottom`/`background`, which show through a translucent window. Its commit (`732aaf1af`) gives no
reason for it on `top`, and no test pins it. Filed as the core entry above.

Real clients: `foot` withheld at `alpha=1.0` and at `alpha=0.5`. I did not confirm that foot's
alpha setting took effect, so this says nothing about foot when translucent. `mpv`, `firefox`,
`chromium` and `imv` are not on the VM, so the buffer shapes a video player or browser uses in
fullscreen were not measured. That matters: any client that fullscreens with an ARGB buffer and no
opaque region lands in the table above.

Also measured: a card already on screen when a fullscreen window arrives is not held. An animating
`top` surface got 112 callbacks in the 2 s before an opaque window covered it and none after, but
its clock had long started. A card shown just before the user goes fullscreen can expire unseen,
the same caveat as lock (`dev/design/scootnotify.md`, Lock). `msg history` is the recovery.

### 3. Cost of a second surface

Five runs per configuration, animating at each callback, one reused buffer per surface, scoot
RSS 2 s after the surfaces mapped minus RSS before (kB):

| Surfaces | scoot RSS delta | client RSS | callbacks per surface in 3 s |
|---|---|---|---|
| `overlay` | 784, 784, 784, 784, 784 | 2244 to 2248 | 170 to 171 |
| `overlay` + `top` | 908, 880, 908, 880, 908 | 2236 to 2248 | 170, 170 |

A second surface is **+96 to +124 kB** in the compositor (one 300x100x4 = 117 kB buffer) and
nothing measurable in the client, with no extra frames per surface. Under an opaque cover the
`top` surface gets none, so a held stack costs nothing per frame. Today's `scootnotify` for scale
(real v0.7.9 binary, `dbus-run-session`, `busctl` notifications at 30 s): RSS 3608 kB idle, 3760
with one card, 4280 with three. (An earlier run in which the probe allocated a buffer per frame
read 31 to 44 MB in scoot; that was the probe's leak, not a cost, and is discarded.)

The memory is not the cost. Splitting the stack is. Cards of both urgencies stack in one corner and
render as one list (newest on top, at most 5 and a "+N more" pill, dismiss-all, hover pause). Two
surfaces anchored to the same corner need margins computed from each other's height, and
layer-shell has no atomic commit across surfaces, so the two stacks can overlap or tear while one
grows. That is real design and test work, well beyond a flag.

### 4. sway 1.12 (headless, pixman)

The rule is not scoot-specific in effect. sway withheld the `top` callback under a fullscreen
window in **both** cases and served it only once the window left fullscreen (at about 2.5 s):

| Window | `top` first callback | `overlay` first callback |
|---|---|---|
| none | 1.1 ms | 1.1 ms |
| `opaque` | 2526.5 ms (after un-fullscreen at ~2.5 s) | 2.7 ms |
| `translucent` | 2528.7 ms (after un-fullscreen at ~2.5 s) | 3.3 ms |

`grim` under the `translucent` window shows the `top` card absent and the `overlay` card drawn.
Hyprland, niri, KDE and GNOME were not available and were not measured, so "other compositors" is
unknown. The hold is a compositor behavior, not part of the protocol (`wl_surface.frame` may be
served for a surface that is not shown), so it must be documented as best-effort.

### Decision

- **Do:** the core fix first, as its own PR. Then a `hold-while-fullscreen` mode for scootnotify
  (off by default, non-critical cards on a `top` surface, critical ones on `overlay`, one merged
  layout across both). Its docs (`site/.../scootnotify/configure.md`) are that PR's, with a
  `> **Symptom:**` box for "a card was gone when I left fullscreen".
- **Do not:** ship the mode before the core fix. On v0.7.9 it silently loses cards under any
  fullscreen window that is not recognised as opaque, which is worse than v1's show-over-fullscreen.
- **Known limit, documented, not fixable by the client:** a card already shown when fullscreen
  starts keeps expiring. Only a fullscreen-state signal would fix that (a foreign-toplevel
  subscription, which this spike deliberately avoided).
- Not pursued: priority stays low, so this stays unscheduled until someone asks for the mode.

### Evidence

Commands (all on the dev VM, `ssh -p 2222 dev@localhost`, in `/var/tmp/fshold`, since removed):
`run.sh WIN LAYERS SECS [anim]` (scenarios 1 to 3), `run2.sh` (screenshots and un-fullscreen),
`run3.sh` (real scootnotify), `run4.sh` (card already shown), `run5.sh` (foot),
`runsway.sh` (sway). Raw lines quoted above are verbatim `FIRST-CALLBACK` and `SUMMARY` output.
Screenshots (not kept): `shot-opaque-covered.png` and `shot-opaque-uncovered.png`,
`shot-translucent-covered.png`, `sway-covered.png`.
