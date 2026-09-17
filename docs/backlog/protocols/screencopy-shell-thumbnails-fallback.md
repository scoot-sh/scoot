---
title: "Shell window thumbnails without a toplevel capture protocol (region crop + quickshell's dmabuf readiness gate)."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Shell window thumbnails without a toplevel capture protocol.

Filed from the phase-1 probe that closed
[`../resolved/screencopy-toplevel-capture-done.md`](../resolved/screencopy-toplevel-capture-done.md)
as unreachable-for-the-motivating-client: stock quickshell 0.3.1 routes a
`Toplevel` `ScreencopyView` capture source exclusively to
`hyprland-toplevel-export-v1` (version-exact source + binary inventory + live
wire evidence in that file), so `ext_foreign_toplevel_image_capture_source_manager_v1`
was never built. This is the fallback that entry names: per-window thumbnails
as a **region capture out of the output** — `wlr-screencopy-unstable-v1` has no
toplevel source either, so there is no other protocol to reach for.

## One measurement to take first (it gates everything, including the overview)

The same probe turned up a second, wider gate: quickshell 0.3.1 never
instantiates *any* capture manager on flexwm — not even for the output path
PR #52 shipped — because `WlBufferManager::isReady()` requires
`mDmabufFormatsReady`, which is set only by real `zwp_linux_dmabuf_v1`
feedback events, which a GPU-less compositor truthfully never sends (full
mechanism in the resolved file). So today **every** quickshell
`ScreencopyView` is blank here: thumbnails for protocol reasons, *and* the
workspace-overview preview for buffer reasons despite its protocol working
(`grim` proves it).

Measure before anything else whether a minimal `zwp_linux_dmabuf_v1`
advertisement can flip that readiness flag on this compositor — *and*
whether such an advertisement can be truthful at all given flexwm has no
render node to describe (if readiness genuinely requires importing dmabufs
the compositor cannot produce, "minimal" is not honest and the answer is
no) — and whether the ext output-capture path then actually displays
over shm (quickshell falls back to shm buffer creation once ready, so in
principle nothing needs a real render node past the flag). If yes, the
overview preview lights up with no further protocol work, and the thumbnail
half below becomes a live question rather than a blank widget. If no — if
readiness genuinely requires importing dmabufs the compositor cannot produce
— then shell thumbnails on flexwm need the shells to change (a shm-only
readiness path upstream), and any compositor-side capture work for them is
moot until that lands. Either answer belongs in the resolved record, the same
way the toplevel probe's NO closed its ticket without a build.

## Then, for the thumbnails themselves

With readiness established, a per-window thumbnail is a client-side crop of
the output capture to the window's rect — the compositor already serves the
pixels (`grim` today), and the window geometry already reaches the shell
(`wlr-foreign-toplevel-management` gives position-relevant state; `flexwm msg
windows` gives exact rects for an agent). What, if anything, flexwm must do
beyond the pixels is the open part: possibly nothing (shell-side crop), which
would make this entry close the same way the toplevel one did — by
measurement, not code.

Explicitly out of scope here: implementing `hyprland-toplevel-export-v1` so
quickshell's existing `Toplevel` path lights up. It is a compositor-specific
protocol, not a standard, so `CLAUDE.md`'s rule points away from it; and it
sits behind the same readiness gate, so it cannot precede the measurement
above. If that measurement lands yes and a later probe shows the Hyprland
path is what DMS/Noctalia thumbnails actually need, file it as its own item
with that justification — do not smuggle it into this one.

Rough size: S for the measurement; unknown for whatever follows it.
