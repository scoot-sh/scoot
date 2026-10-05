---
title: "Desktop clipboard: persistence, history picker, lock behavior"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Desktop clipboard: persistence, history picker, lock behavior

Filed 2026-10-04, child 8 of `desktop-paved-path`, required slot per
maintainer addendum 2026-10-04 ("make sure we have clipboard support").
Serves **daily-drive** (copy dying with the source app is data loss from the
user's point of view) and **computer use** (agents move text through the
clipboard constantly).

## The gap (compositor side verified, desktop side missing)

Verified in `docs/protocols.md`: `zwlr_data_control_manager_v1` v2 with
`cliphist`/`clipman` named as clients (31, 2083-2085),
`ext_data_control_manager_v1` v1 side by side with it (32, 2086-2088),
focus-gated `zwp_primary_selection_device_manager_v1` (33, 2089-2093), and
the XWayland clipboard crossing rules (~554-558, ~805). So the compositor
serves core `wl_data_device`, both data-control generations, and primary
selection. The desktop half does not exist: no manager, no
`wl-copy`/`wl-paste` on PATH, no history picker bind — copy currently dies
with the source app.

## What to do

Fill the `desktop.clipboard` slot (`cliphist` + `wl-clipboard` as the
default pair — measure against clipman, record closure, say why):

- Persistence: a manager holding the clipboard after the source app closes
  (the data-control manager exists for exactly this; say which generation
  the pick speaks and why both being exposed matters).
- History with a picker bound to a key, through the launcher slot's dmenu
  contract (child `desktop-launcher`: lines on stdin, selection on stdout)
  — not a second picker UI.
- `wl-copy`/`wl-paste` on PATH for scripts and terminals.
- Primary selection: middle-click paste keeps working through the manager
  (or is explicitly left compositor-native — decide, say why; the
  focus-gate in `protocols.md:2089-2093` constrains the shape).
- Lock behavior: no history capture and no picker while the session is
  locked (state the mechanism — clear-on-lock vs refuse-while-locked).
- Sensitive entries: password managers' `x-kde-passwordManagerHint`
  (and the equivalent concealment flags) must exclude entries from history
  — name the exact MIME/flag checks, with a test using a real secret tool.

Acceptance: eval pins in `nix/tests.nix`; a real copy → close the source
app → paste test on the M2 (clipboard survives), history picker round-trip
(copy three things, pick the second), a lock test (copy, lock, unlock —
history policy holds), and a secrets test (a password-manager copy never
lands in history); docs in `docs/nix.md`.

## Not in this ticket

Clipboard sync across machines; image-history size tuning beyond a sane
default cap (say the cap); primary-selection persistence if left native
(state it, don't gold-plate).
