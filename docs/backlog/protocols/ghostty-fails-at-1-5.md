---
title: "Ghostty fails to load at `[output] scale = 1.5` (works at 2.0); the integer `wl_surface.preferred_buffer_scale` was missing and is now sent, but that does not explain the symptom"
status: "open"
area: "protocols"
priority: "high"
blocked: "needs the reporter's Asahi machine to confirm or refute the fix"
---

# Ghostty fails to load at `[output] scale = 1.5` (works at 2.0)

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14, right after
`[output] scale` (PR #30) landed:

- `[output] scale = 2.0` → Ghostty loads and renders correctly.
- `[output] scale = 1.5` → **Ghostty does not load.**
- `foot` works at both `1.5` and `2.0`.

## What was found and fixed (a real, separate protocol gap)

Output scaling created the `wl_compositor` global with `CompositorState::new`
(version 5). `wl_surface.preferred_buffer_scale` is a **v6** event, and
Smithay's `send_surface_state` (`wayland/compositor/mod.rs:411` in the pinned
rev) early-returns for `version() < 6` and has no caller anywhere in Smithay.
So a client that opts into `wp_fractional_scale_v1` received the exact
fractional `preferred_scale` but never the integer companion event.

That gap is real and is now closed (PR #31): `CompositorState::new_v6` plus a
`send_surface_state` call from `CompositorHandler::new_surface`, so a v6
client receives **both** `preferred_scale` (`1.5`) and
`preferred_buffer_scale` (`ceil(1.5) = 2`). Regression-tested on the dev VM
with a real client, red/green.

## Why this entry is still open, not resolved

**The fix does not explain the reported symptom, and review found evidence it
may not fix it.** GTK4's own source
(`gdk/wayland/gdksurface-wayland.c`, `surface_preferred_buffer_scale`) returns
early and ignores the event whenever a `wp_fractional_scale_v1` object exists —
which it always does now that flexwm advertises that global. So a GTK4/Ghostty
client likely discards the very event this PR adds.

What that means:

- The integer companion was worth sending for protocol completeness (it is
  what wlroots does), and that part is correct and tested.
- It is **not established** that sending it makes Ghostty load at `1.5`. The
  root cause of the reported failure is therefore **unknown** as of this entry.

## Local reproduction attempt (2026-09-14) — did NOT reproduce

Attempted on the dev VM rather than guessing, per step 3 above. Ghostty **is**
installable there (`nix-shell -p ghostty` → 1.3.1). With `main` at `6cf1106`
(includes #30 and #31), under `--headless` at a virtual output:

| client | scale 1.5 | scale 2.0 |
| --- | --- | --- |
| `ghostty --gtk-single-instance=false` 1.3.1, kept alive | **maps** (logical 409×510) | maps (360×376) |
| `gtk4-demo` (GTK 4.22.4) | maps | maps |
| `foot` | maps | maps |

Ghostty rendered real content at 1.5 (a 1280×800 screenshot at scale 1.5 was
35,623 bytes of drawn frame, not a blank backdrop). The logical rects shrinking
as scale rises (409×510 at 1.5 vs 360×376 at 2.0; a fresh run gave 302×376 at
2.0 with a different window title/decoration) shows GTK4/Ghostty are honoring
the fractional scale — flexwm's protocol side is delivering.

**So this is not reproducible on the dev VM, and the earlier theory that it is
output-scaling's fault is unsupported.** Three harness artifacts during this
investigation each looked like the bug and were not:

- A fixed 4–5 s wait was too short for GTK4/Ghostty to map under software GL
  (they need ~7 s; foot needs ~0.5 s) — "no windows mapped" was timing.
- A first binary tested predated the output-scaling feature — an apples-to-
  oranges comparison that briefly suggested a pre-existing GTK4 bug.
- `ghostty -e true` exits immediately, so the window opens and closes in a
  fraction of a second; polling after it was gone read as "doesn't load" at
  both scales. Keeping the process alive (`-e sh -c "sleep 60"`) makes it
  deterministic.

Anyone re-running this must **poll for the window** and **keep the client
alive**; a fixed sleep plus a command that exits will produce a false failure.

## What this narrows it to

The failure is specific to the reporter's machine, so the distinguishing
variable is not "GTK4 at 1.5" in general. Candidates, roughly:

1. **Ghostty version.** Dev VM has 1.3.1. An older or newer Ghostty may have
   different fractional-scale behavior; the reporter's version is unknown.
2. **The GPU/GL path.** The dev VM falls back to software (Mesa `swrast`/
   `zink` failures are all over its logs); the Asahi M2 runs a real
   GPU-accelerated GL/EGL path. A fractional buffer allocation failure in
   Ghostty's GPU renderer would not reproduce under software rendering —
   this is the leading hypothesis, and it is the one thing the VM structurally
   cannot exercise.
3. **`wl_output` version or other global differences** the VM's flexwm also has,
   so less likely.

## What to do next

1. **Ask the reporter for their Ghostty version** (`ghostty --version`) and,
   at 1.5, `WAYLAND_DEBUG=1 ghostty` stderr — specifically the lines after the
   first `wl_surface.commit`, and any EGL/GL error. That distinguishes (1)
   from (2) immediately.
2. If it is the GPU path, the tell will be a Ghostty-side EGL/GL error rather
   than a Wayland protocol error — in which case flexwm's scaling is not the
   defect and the fix (if any) is Ghostty-side or a workaround
   (`scale = 2.0`, or Ghostty's own `window-scale`/font-size setting).
3. Do not ship a compositor change for this until the stderr names a flexwm
   defect. The protocol work (#30, #31) stands on its own merits regardless.

## Related

- `docs/backlog/resolved/output-scaling-done.md` — the scaling feature itself.
- `docs/backlog/resolved/fractional-scale-integer-companion-done.md` —
  the integer-companion mechanism, fixed and tested; kept there for the
  diagnosis even though this symptom it was thought to explain did not
  resolve.
