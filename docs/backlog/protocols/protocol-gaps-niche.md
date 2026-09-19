---
title: "Niche protocol gaps, lowest priority for this project's current scope, bundled for the same reason as the entry above."
status: "resolved"
area: "protocols"
priority: "low"
blocked: null
---

# Niche protocol gaps, lowest priority for this project's current scope, bundled for the same reason as the entry above.

**Niche protocol gaps, lowest priority for this project's current scope,
  bundled for the same reason as the entry above.** User request,
  2026-09-13 — **TRIAGED PER SUB-ITEM 2026-09-18, see
  `../resolved/protocol-gaps-niche-done.md`:**
  - **`tablet-v2`** — drawing-tablet (Wacom-style) input support. **Filed
    separately** as `../input/tablet-v2.md`, since implemented and
    recorded as [`../resolved/tablet-v2-done.md`](../resolved/tablet-v2-done.md):
    an input epic (libinput plumbing, `TabletSeat`, tool focus/cursor),
    not a bare advertisement.
  - **`wlr-screencopy-unstable-v1`/a newer `ext-image-copy-capture-v1`** —
    ~~lets third-party tools (`grim`, `wf-recorder`, screen-sharing in video
    conferencing apps) capture the screen directly, rather than going
    through scoot's own bespoke `scoot msg screenshot` IPC action. Worth
    revisiting against `CLAUDE.md`'s "prefer the standard protocol over a
    bespoke one" rule at some point — scoot's own screenshot action
    exists because computer-use automation needs it under scoot's own
    control/auth model, but that doesn't mean third-party tools shouldn't
    also have the standard path available to them.~~ — **ALREADY RESOLVED**
    (PR #52, `../resolved/screencopy-capture-done.md`); the wlr half stays
    deliberately unbuilt.
  - **`wlr-output-management-unstable-v1`** (or a newer successor) — lets
    tools like `wlr-randr`/`kanshi` query and reconfigure output mode,
    position and scale. Moot while scoot has exactly one `Output` and no
    real multi-monitor support; relevant once that lands. *Half-resolved
    2026-09-16 (PR #49): the query half shipped — it turned out to have real
    clients (shell display pages) well ahead of multi-output support, and
    there is no newer successor to prefer. See
    `../resolved/output-management-read-only-done.md`. The reconfigure half
    is still deliberately refused — closed 2026-09-18 as an accepted
    tradeoff rather than a deferred defect (no authorization concept in the
    protocol, per-backend honesty gaps, no shell demand), and is recorded as
    `../resolved/output-management-reconfiguration-done.md`.*
  - **`security-context-v1`** — lets a compositor scope what a sandboxed
    client (e.g. a Flatpak) is allowed to do. Relevant for hardened setups,
    not a natural fit with this project's current minimalist scope.
    **Closed as a deliberate non-entry** 2026-09-18 (no sandbox machinery
    or demand; an allow-list without it would be theatre).
  - **`content-type-v1`, `alpha-modifier-v1`** — ~~minor rendering hints (a
    client declaring "I'm showing video/a game," or setting whole-surface
    opacity without compositing it itself). Low value on a CPU/pixman-only
    renderer with no adaptive-sync or GPU compositing story.~~ —
    **IMPLEMENTED** 2026-09-18: verify-first split the pair (alpha has a
    real in-tree consumer, content-type is stored and honestly ignored).
    See `../resolved/protocol-gaps-niche-done.md`.

**From `flexwm-reviewer's pass on PR #13 (item 8, client cursor surface
rendering), all low priority, none blocking:**

- **`Cursor::element`'s fallback path allocates a one-element `Vec` every
  rendered frame (LOW).** The old code returned `Option<...>` with no
  allocation; now every `--tty` frame showing the default cursor (the
  common case) allocates and frees a `Vec` to hold it. Not observable in
  benchmarking (12+4 interleaved reps, no measurable difference), and
  `render()` already builds a few per-frame `Vec`s this way, so it matches
  local convention rather than breaking it — but a cheaper shape exists if
  it ever matters: have `element()` append into a caller-owned
  `&mut Vec<CursorElement<R>>` instead of returning a fresh one; a full fix
  (a persistent element buffer on `Backend`) is a larger refactor than fits
  here. The `Surface` path allocates regardless of this fix, since
  Smithay's own `render_elements_from_surface_tree` returns a `Vec`.
  **Re-derived 2026-09-18 and closed as a deliberate non-entry** (shape
  unchanged, convention unchanged, `Surface` path still allocates per
  frame regardless).
- **An animated cursor client keeps getting woken while the `--tty` session
  is VT-paused (LOW, inherited not introduced).** `Tty::present` early-
  returns on `!active`, but `render()` and the frame-callback loops run
  regardless — identical to the pre-existing per-window `send_frame` loop;
  item 8 just makes the cursor share it. Not new, not specific to cursors.
  **ALREADY RESOLVED** (`../resolved/cursor-frame-callback-when-paused-done.md`).
- **`wl_surface.offset` on a cursor surface doesn't move the hotspot (LOW,
   upstream gap).** Per `wayland.xml`, `hotspot_x`/`hotspot_y` should
   decrement on `wl_surface.offset` requests to a cursor surface. At the
   pinned Smithay rev, `CursorImageAttributes.hotspot` is only ever written
   by `wl_pointer.set_cursor` (and the tablet-tool equivalent) — nothing
   adjusts it on offset/commit — and scoot reads it verbatim. A client
   using `wl_surface.offset` on its cursor gets a misplaced image. Not a
   regression (nothing rendered for `Surface` before item 8), and real
   toolkits don't appear to do this in practice. **Re-verified at the
   pinned rev 2026-09-18 and closed NEEDS-UPSTREAM** (the fix belongs in
   Smithay's set_cursor/commit path). **OVERTURNED 2026-09-18 — fixed
   scoot-side** ([record](../resolved/cursor-surface-offset-hotspot-done.md)):
   the NEEDS-UPSTREAM half-truth was that *core* never adjusts it;
   Smithay's own anvil does the decrement compositor-side in its shell
   commit hook, which is exactly where scoot's `Cursor::note_surface_commit`
   now does it (saturating).

(Note: this file always ended mid-sentence at "none blocking:" -- the
reviewer items above were recovered from the pre-split `ROADMAP.md`, `git
show b00d2ff^:ROADMAP.md`, and restored here as part of closing the
bundle.)
