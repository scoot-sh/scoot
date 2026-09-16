---
title: "Niche protocol gaps, lowest priority for this project's current scope, bundled for the same reason as the entry above."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Niche protocol gaps, lowest priority for this project's current scope, bundled for the same reason as the entry above.

**Niche protocol gaps, lowest priority for this project's current scope,
  bundled for the same reason as the entry above.** User request,
  2026-09-13:
  - **`tablet-v2`** — drawing-tablet (Wacom-style) input support.
  - **`wlr-screencopy-unstable-v1`/a newer `ext-image-copy-capture-v1`** —
    lets third-party tools (`grim`, `wf-recorder`, screen-sharing in video
    conferencing apps) capture the screen directly, rather than going
    through flexwm's own bespoke `flexwm msg screenshot` IPC action. Worth
    revisiting against `CLAUDE.md`'s "prefer the standard protocol over a
    bespoke one" rule at some point — flexwm's own screenshot action
    exists because computer-use automation needs it under flexwm's own
    control/auth model, but that doesn't mean third-party tools shouldn't
    also have the standard path available to them.
  - **`wlr-output-management-unstable-v1`** (or a newer successor) — lets
    tools like `wlr-randr`/`kanshi` query and reconfigure output mode,
    position and scale. Moot while flexwm has exactly one `Output` and no
    real multi-monitor support; relevant once that lands. *Half-resolved
    2026-09-16 (PR #48): the query half shipped — it turned out to have real
    clients (shell display pages) well ahead of multi-output support, and
    there is no newer successor to prefer. See
    `../resolved/output-management-read-only-done.md`. The reconfigure half
    is still moot for exactly the reason above, and is re-filed as
    `output-management-reconfiguration.md`.*
  - **`security-context-v1`** — lets a compositor scope what a sandboxed
    client (e.g. a Flatpak) is allowed to do. Relevant for hardened setups,
    not a natural fit with this project's current minimalist scope.
  - **`content-type-v1`, `alpha-modifier-v1`** — minor rendering hints (a
    client declaring "I'm showing video/a game," or setting whole-surface
    opacity without compositing it itself). Low value on a CPU/pixman-only
    renderer with no adaptive-sync or GPU compositing story.

**From `flexwm-reviewer`'s pass on PR #13 (item 8, client cursor surface
rendering), all low priority, none blocking:**
