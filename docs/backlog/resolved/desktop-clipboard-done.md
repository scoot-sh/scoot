---
title: "Desktop clipboard: persistence, history picker, lock behavior"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
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

## Resolution (PR #443, 2026-10-05)

Landed as specified: `desktop.clipboard` on with the profile (still
individually disable-able), a lean cliphist (stock nixpkgs drags ~208
MiB of contrib-picker fat for pickers the profile never runs; the lean
build is 2.5 MiB marginal) watched by `wl-paste` (one user unit per
selection into one history, `ext-data-control-v1` first with the `wlr`
v2 fallback -- which is why both generations stay exposed),
`wl-copy`/`wl-paste` on PATH, and the history picker on the reserved
`Super+v` (cliphist through fuzzel's dmenu mode -- lines on stdin,
selection on stdout, the contract the launcher child reuses -- themed
by the look through CLI flags, `theme.targets.clipboard.enable` opts
out). Lock policy is both halves: clear-on-lock (the history wiped
before the locker on swayidle's `lock` and `before-sleep` lines) and
refuse-while-locked (the store entry and the picker probe the
compositor with a side-effect-free `focus-window-id` for an unreachable
id, failing open without IPC). Secrets: only `x-kde-passwordManagerHint`
is checked (presence-based, the value unchecked -- verified in
wl-clipboard's source at the pinned rev), so managers that don't set it
are not excluded, stated plainly. Primary selections are captured into
the same history while middle-click stays compositor-native (the focus
gate). History keeps 100 entries (5 MB each max, cliphist's own cap) in
`~/.cache/cliphist/db` by default, surviving reboots with secrets never
landing in it; the live selection still dies with its owner (measured,
not assumed -- one picker Enter restores it).

Evidence: `nix/tests.nix` eval pins (both sides, refusals, Darwin
nulls), stub-tool behavior tests for the entry and the picker (cancel,
wipe-race, lock, fail-open, byte-exact restore), content checks
(wipe lines, theme flags, lean closure) -- `checks.aarch64-linux` and
`checks.aarch64-darwin.scoot-modules` both exit 0 on the head; live
proof in a `scoot-test` login on the Asahi M2 (machine scoot, this
tree's scripts): copy, close foot, live dead but history kept; picker
round-trip through real fuzzel driven over IPC; `wl-copy --sensitive`
absent from history with the hint verified on the live offer; locked:
binds refused, guard drops, picker refuses, wipe empties; typed-password
unlock resumes capture; middle-click pastes live. Watchers 1904/1920 KB
RSS, 0 wakeups over 60 s idle. Docs in `docs/nix.md` ("Clipboard",
with the measured pick table and an IPC screenshot). Considered and
declined: `wl-clip-persist` for live-ownership (unasked scope, and it
would hold sensitive selections live) and a reboot-persistence toggle
(documented trade-off instead).
