---
title: "Niche protocol gaps triaged per sub-item: alpha-modifier + content-type implemented, tablet filed separately, the rest closed deliberate/needs-upstream/already-resolved — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Niche protocol gaps — DONE (triaged per sub-item)

The niche bundle (`docs/backlog/protocols/protocol-gaps-niche.md`) held
nine sub-items: five from the 2026-09-13 user request and four from
`flexwm-reviewer`'s pass on PR #13 (item 8) -- the ticket file itself ends
mid-sentence at "none blocking:", and the rest was recovered from the
pre-split `ROADMAP.md` (`git show b00d2ff^:ROADMAP.md`, lines 3599-3640).
Each got the minimum honest disposition, verify-first: checked against the
pinned Smithay rev (`0ff00983`, in source, not from general Smithay
knowledge), the protocol XMLs, and real-client demand (the DMS/Noctalia
probe records plus live `foot`/`wayland-info` on the dev VM). Two were
implemented, one was filed as its own entry, six closed without code.

## What landed

- **`wp_alpha_modifier_v1`** (version 1). `compositor/alpha_modifier.rs`.
  A client-controlled whole-surface opacity factor. Smithay carries the
  whole protocol at the pinned rev, and -- the load-bearing check --
  `WaylandSurfaceRenderElement::from_surface`
  (`backend/renderer/element/surface.rs:272-273`) multiplies the committed
  factor into every surface-tree element it builds, while the pixman
  backend draws a sub-1.0 element through a solid-alpha mask
  (`backend/renderer/pixman/mod.rs:610-611`). flexwm's windows, layer
  surfaces, lock surfaces and client cursor surfaces all build through
  that same `from_surface`, so all four honour the factor with zero
  flexwm-side render work: two hold-alive fields' worth of implementation
  (one here, one for content-type below).
- **`wp_content_type_manager_v1`** (version 1).
  `compositor/content_type.rs`. A client labeling what kind of pixels a
  surface holds (`photo`/`video`/`game`/`none`). Stored by Smithay and
  read by nothing: no backend code at the pinned rev consumes the cached
  state, and a CPU/pixman renderer with no adaptive-sync or GPU story has
  no consumer to give it -- so the implementation is advertisement plus
  the honest documentation that no pixel changes, pinned byte-identical
  rather than asserted on a stored field.

Five fail-first harness tests for alpha (advertised, `u32::MAX` opaque,
half blends with the background, modifier-destroy restores opacity,
manager-destroy leaves surfaces working) and three for content-type
(advertised, all four types render byte-identical, manager-destroy leaves
objects working) -- each confirmed to fail with the globals unadvertised
before being kept. No bind-budget, pool-budget or buffer-budget
interaction on either path (no pool, buffer or fd is created), so none of
the three bounds moves.

## Per-sub-item verdicts

1. **`tablet-v2` -- FILED SEPARATELY** as
   `docs/backlog/input/tablet-v2.md`, since implemented and recorded as
   `docs/backlog/resolved/tablet-v2-done.md`. Advertising `TabletManagerState`
   alone would be the dishonest version: clients would bind it and get
   zero tools, because flexwm has no tablet input path (no libinput
   tablet-event plumbing, no `TabletSeat`, no tool focus/cursor
   integration). That is an input epic, not a three-line advertisement,
   and neither probed shell binds the global. The new entry sizes it.
2. **`wlr-screencopy` / newer `ext-image-copy-capture-v1` -- ALREADY
   RESOLVED**, no work. The `ext-` half shipped as PR #52
   (`screencopy-capture-done.md`); the wlr half stays deliberately
   unbuilt, the opposite call to PR #50 for the same reason (measured:
   `grim` 1.5.0 and quickshell 0.3.1 speak only the `ext-` one).
3. **`wlr-output-management` -- ALREADY RESOLVED**, no work. Query half
   shipped as PR #49 (`output-management-read-only-done.md`);
   reconfiguration closed as a deliberate refusal
   (`output-management-reconfiguration-done.md`).
4. **`security-context-v1` -- DELIBERATE NON-ENTRY**, no code. Smithay
   carries it at the pinned rev, but advertising it honestly needs a
   sandbox story flexwm has none of: the filter must exclude
   context-created clients "for the protocol to be correct and secure"
   (pinned rev `security_context/mod.rs`), and the compositor must then
   actually scope what sandboxed clients may do -- there is no such
   machinery, no Flatpak/sandbox demand in either probe record, and the
   ticket's own "not a natural fit with this project's current minimalist
   scope" still holds. An allow-list without it would be theatre, the same
   call every other advertisement's trust note already makes.
5. **`content-type-v1` -- IMPLEMENTED** (above). The ticket's "low value
   on a CPU/pixman-only renderer" is still true and now documented on the
   tin: the value is future-proofing plus protocol completeness, at three
   lines plus tests.
6. **`alpha-modifier-v1` -- IMPLEMENTED** (above). The ticket grouped it
   with content-type under "low value", but verify-first split the pair:
   unlike the hint, the factor has a real consumer already in-tree
   (Smithay's `from_surface` times the pixman mask), so this one works
   end to end rather than merely advertising.
7. **`Cursor::element`'s one-element fallback `Vec` (LOW) --
   DELIBERATE NON-ENTRY**, no code. Re-derived, not assumed: `element()`
   still returns `Vec<CursorElement<R>>` with a `vec![...]` on the
   fallback path, but so does the `Surface` path on every frame (Smithay's
   own `render_elements_from_surface_tree` returns a `Vec`), and
   `render()` builds fresh per-frame `Vec`s for windows and layers the
   same way -- the cheaper shape (append into a caller-owned buffer)
   would ripple the signature for an allocation that matches local
   convention and measured nothing when the review filed it.
8. **Cursor frame callbacks while VT-paused (LOW) -- ALREADY
   RESOLVED**, no work. `cursor-frame-callback-when-paused-done.md`:
   `render()` and the frame-callback loops are gated on DRM master.
   Still present in `headless.rs::render` (the `Tty::active` gate with
   its deliberate-`active`-not-`session_paused` comment).
9. **`wl_surface.offset` not moving the cursor hotspot (LOW) --
   NEEDS-UPSTREAM**, no code. Re-verified at the pinned rev: Smithay
   applies `offset` to the surface's `buffer_delta`
   (`wayland/compositor/handlers.rs:321-331`) and never to
   `CursorImageSurfaceData.hotspot` (written only by `set_cursor`),
   while flexwm reads the hotspot verbatim. The fix belongs in
   Smithay's set_cursor/commit path, and no probed toolkit does this in
   practice (the ticket's own qualifier).

## Evidence

Fail-first runs (dev VM, `XDG_RUNTIME_DIR=/tmp/xdgrt cargo test -p
flexwm -- alpha_modifier content_type`, unmodified `State`): all 8 fail
-- the two advertisement tests on the missing globals, the rest on the
client giving up waiting for them. Same command after the two `State`
fields land: 8 pass.

Full standard set on the dev VM after the implementation (exact commands
and raw outputs in the PR report): `cargo test -p flexwm`, `cargo
nextest run --workspace`, `cargo clippy -p flexwm --all-targets -- -D
warnings`, `cargo fmt --check -p flexwm`, `scripts/smoke-test.sh`.

Live advertisement (dev VM, `--headless`, at the implementation commit):
`wayland-info` lists `wp_alpha_modifier_v1` and
`wp_content_type_manager_v1`; `WAYLAND_DEBUG=1 foot` maps and renders
with neither warning nor disturbance -- `foot` binds neither global
(it has no use for either), which bounds what the advertisement changes
today, the same way the `foot` record bounded the cursor-theme work.

## What this deliberately leaves open

- **`tablet-v2`** was its own entry (`docs/backlog/input/tablet-v2.md`)
  rather than a line in this bundle, and is now implemented and recorded
  as `docs/backlog/resolved/tablet-v2-done.md`.
- **Content-type has no reader.** If a future GPU tier grows one
  (overlay planes, adaptive-sync timing), the stored value is already
  where it looks; until then the README says ignored, not "used".
- **No per-surface alpha in IPC.** `flexwm msg windows` does not report
  the factor and no action sets it -- an agent reads pixels, not
  protocol state, and no agent loop asked for it.
