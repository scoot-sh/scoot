---
title: "Seat reconnect for the GPU scanout tier (teardown-first rebuild of scanout heads)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Seat reconnect for the GPU scanout tier (teardown-first rebuild of scanout heads)

Filed 2026-10-10. Serves **daily-drive**: the lost-master recovery that
ships for pixman sessions today must also cover GPU sessions — otherwise
flipping `--tty` to GPU-first would silently remove the #548 recovery
from every default real-hardware session, which under CLAUDE.md's "never
defer a user-facing harm" rule blocks stage C. High priority for exactly
that reason: it gates the default flip.

## The gap

Seat reconnect (`crates/scoot/src/compositor/tty/reconnect.rs`) is
pixman-only. After the arm/gate checks, `run` returns early on any
non-pixman session: "a scanout rebuild would re-tie renderer state this
module does not touch. Nothing has been torn down at this point, so
returning keeps today's path whole" (`reconnect.rs`, the `tty.renderer()
!= RendererKind::Pixman` early return with its `seat reconnect not
attempted` INFO line). A GPU session that loses the VT-switch-back master
race therefore sits at `live: false` until the next switch back wins the
race, while a pixman session rebuilds and recovers (route (a) ships, 5/5
lost races recovered live on the M2 —
`docs/backlog/resolved/seatd-reconnect-or-close-to-zero-done.md`). The
pixman path rebuilds through `build_heads(&mut tty.drm, &drm_fd,
connected, Some(wanted), wanted)` on the fresh fd and rewires input
(`reconnect.rs:585-600`); nothing equivalent exists for
`ScanoutPresenter`s/`ScanoutBackend`s, and no `Backend::replace_scanout`
handoff exists.

## What to do

Extend the teardown-first rebuild to the scanout tier, following the
#548 discipline (`reconnect.rs:21-46` module doc): refuse while a flip is
in flight or retry from idle once `flip_tracker` reports none; drop the
old presenters and pipelines before building the new ones; on any
failure rebuild the old tier, never leave a dark session. Concretely:
drop each head's `ScanoutPresenter` + `ScanoutBackend` first, build a new
`GbmDevice` on the fresh fd, `try_scanout` per head, and re-handoff to
the existing `Backend`s via a new `Backend::replace_scanout`. Prove
master synchronously by commit, the way the pixman path does. Resolve
from the session's `RendererKind` the way reconnect already does
(`reconnect.rs:129` reads `state.renderer`), so this runs in parallel
with `renderer-auto-policy` and rebases onto whichever lands first — no
dependency on the `auto` request. Hotplug's resolved-tier mapping
(`hotplug.rs:651`) is the model for the per-head plan.

**Evidence:** its own Asahi live matrix like #548 — vacant-switch-back
lost races recovered on the GPU tier, with the exact commands, SHA, and
raw logs (INFO lines, `scanout="gpu"` after rebuild, `scoot msg outputs`
`live: true`). The dev VM can exercise the negative/refusal paths only
(single-GPU virtio box); the positive path needs the M2. Qt/QML-free runs:
`pgrep -a 'scoot|seatd|openvt|sway|niri'` before claiming the seat, M2 via
`~/fx/vt-run.sh` / `~/fx/vt-stop.sh`, `fgconsole = 1` and clean
`loginctl` after.

## Not in this ticket

The `auto` policy itself (`renderer-auto-policy`); the runtime IPC swap
(`runtime-renderer-swap`, which reuses this rebuild machinery for
`--tty`); close-to-zero (unneeded and untried, stays that way unless this
route fails live); any change to the pixman reconnect path (regression
suite must stay green untouched).
