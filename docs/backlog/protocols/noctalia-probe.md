---
title: "Noctalia enablement probe — results 2026-09-14."
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# Noctalia enablement probe — results 2026-09-14.

Probe (not a build): does Noctalia (the other Quickshell-based desktop
shell — bar, notifications, launcher, lock screen, wallpaper; upstream
`noctalia-dev/noctalia-shell`, GPL-3.0 — nothing copied, so the license
only matters for future asset/code borrowing) fare better than DMS under
flexwm? Baseline: the DMS probe report
(`docs/backlog/protocols/dms-enablement-gaps.md`, 2026-09-14) plus PR #34
(the layer-surface post-destroy commit kill fix, merged on `main`).

Result: **Noctalia runs and renders fully, and the PR #34 fix holds for
this second, independent Quickshell client — 14/14 overlay dismissals
survive, including the exact `destroy` + `attach(nil)` + `commit`
teardown that killed DMS 4/4. But unlocking the session kills the shell
1/1 through a *different* teardown path** (gap 1 below): the
`ext-session-lock` surface teardown trips `CommitBeforeFirstAck`, the
already-filed `session-lock-post-destroy-commit` bug — this probe is
the real-quickshell confirmation that entry was waiting for, and it
raises the entry from "hardening?" to P0.

## How this was measured

- flexwm at `2257696` (current `main`, post-#34 incl. the review
  follow-up), rebuilt in the dev VM
  (`/var/cargo-target/debug/flexwm`, `cargo build` from the 9p mount —
  incremental, exits clean, target dir guest-local).
- `flexwm --headless --width 1600 --height 900 --socket
  /run/user/1000/flexwm-noctalia.sock` (Wayland `wayland-1`).
- Noctalia from nixpkgs, ephemeral only, no VM mutation:
  `nix shell nixpkgs#noctalia-shell nixpkgs#quickshell -c noctalia-shell`
  (noctalia-shell 4.7.7, quickshell 0.3.1 — the same quickshell DMS used,
  so no client-version skew; `QT_QPA_PLATFORM=wayland`). The
  `noctalia-shell` bin is a small C launcher that sets
  `QT_PLUGIN_PATH`/`NIXPKGS_QT6_QML_IMPORT_PATH`/`XDG_DATA_DIRS` and
  execs `noctalia-qs` (0.0.12) with `-p <store>/share/noctalia-shell`.
  (Nixpkgs lists the homepage as `noctalia-dev/noctalia`; the task brief
  says `noctalia-dev/noctalia-shell` — the repo was renamed at some
  point. No functional impact.)
- Driven via `flexwm msg` (`screenshot`, `type`, `key`, `pointer click`)
  and Noctalia's own IPC (`noctalia-shell ipc call <target> <fn>` — 31
  targets, see below); screenshots pulled to the Mac and viewed.
  Advertised globals cross-checked with `wayland-info` (18 globals —
  the same set as the DMS probe). One full session under
  `WAYLAND_DEBUG=1` (`/tmp/noctalia-wire.log`, ~36k lines) plus server
  log; every claim cites the command plus the log signature.
- First launch is slow (~60s before surfaces map: config generation,
  plugin scan, shader/fontconfig warm-up — the first screenshot came
  back the all-black 30046-byte frame while the process was still
  initializing). Second start renders in ≤20s. Not a compositor issue;
  noted so the black frame isn't misread.
- Screenshots: local `/tmp/noc-*.png` on the Mac during the probe, not
  committed. VM left clean (probe processes killed, temp sockets/logs
  and `~/.config/noctalia` removed).

## What runs today

| Noctalia module | Verdict | Evidence |
| --- | --- | --- |
| Bar + wallpaper (layer-shell Top + Background) | works | `noctalia-bar-content-headless`, `noctalia-bar-exclusion-top-headless`, `noctalia-background-headless` mapped (wire log); clock, workspace pill, tray icons, owl wallpaper render |
| Workspace indicator | works — live | bar pill shows `1`; ext-workspace group/output_enter/workspace `1`/active consumed generically (wire + qslog) |
| Launcher overlay | works, incl. repeated dismiss | `ipc call launcher toggle` ×6 open/close; search box live-filters on `msg type "foot"` (6→3 results, real VM .desktop entries); `Escape` dismiss; shell survives all |
| Calendar panel | works until toggled shut | Sept 2026 grid, 14 highlighted, weather placeholder; open+close survives |
| Settings window | works, fully rendered | General→Basics, full sidebar (Bar, Dock, Lock Screen, Session Menu, Idle, Audio, Display…); open+close survives |
| Control Center, notification history | toggled, shell survives | open+close cycles, no kill (visual open-state verified for calendar/settings/launcher; CC/history verified toggle-safe) |
| First-run modals (Privacy, What's-new) | dismiss via pointer, survive | two `msg pointer click` dismissals; pointer input reaches layer surfaces |
| Real notification delivery | works | `notify-send` over D-Bus renders a toast ("Hello from Noctalia probe") |
| Lock screen (`ext-session-lock-v1`) | locks, auth works, **unlock teardown kills the shell (gap 1)** | lock surface renders ("Welcome back, flexwm developer!", password field, session buttons); PAM for user `dev` → `Authenticated successfully`; `unlock_and_destroy` → client killed |
| xdg-toplevel coexistence | works | `msg action spawn foot` maps, tiles, and renders behind/around overlays while the shell runs |
| Keyboard focus to overlays | works | typing reaches the keyboard-interactive launcher; `Escape`/password entry reach lock surfaces |

Noctalia IPC surface (for the record — materially richer than DMS's):
`airplaneMode bar battery bluetooth brightness calendar cb colorScheme
controlCenter darkMode desktopWidgets dock idleInhibitor launcher
location lockScreen media monitors network nightLight notifications
plugin powerProfile sessionMenu settings state systemMonitor toast
volume wallpaper wifi` — 31 targets (`ipc show`).

## Dismiss-survival verdict (the highest-value question)

**The PR #34 fix holds for a second, independent Quickshell client:
14/14 layer-surface dismissals survive.** Tally: 2 pointer-click modal
dismissals + 5 launcher toggle-close cycles + 1 launcher `Escape`
dismiss + 2 calendar + 2 control-center + 2 settings toggles. Wire log
shows zero `The Wayland connection broke`, zero `delete_id` cascades;
the shell's pid is unchanged across all of them and screenshots stay
live. This confirms the destroy-kill fix isn't DMS-specific — Qt's
standard `destroy` + `attach(nil)` + `commit` teardown is now safe on
the layer-shell path.

Two logging notes for future triage: (a) five `discarded
[unknown]#N.[event 0]` lines appear at startup (frame callbacks dropped
in the initial configure round-trip) with zero consequence — same wire
shape as the DMS kill signature but *without* the `delete_id`
cascade/connection break, so discards alone don't mean a kill; (b) every
transient `noctalia-shell ipc …` invocation logs a flexwm
`wayland client disconnected` line (it connects to `WAYLAND_DISPLAY`
and exits), so disconnect lines need pid correlation before they mean
anything.

## Gap list (delta vs. DMS)

### 1. Unlocking the session kills the shell: `CommitBeforeFirstAck` on the lock-surface teardown (P0 — confirms and escalates the existing `session-lock-post-destroy-commit` entry)

- **This is the bug already filed as
  `docs/backlog/protocols/session-lock-post-destroy-commit.md` — this
  probe is the field confirmation that entry was waiting for.** That
  entry's candidate resolution (a) asks to "confirm what quickshell's
  real unlock teardown sends — if it never commits after role destroy,
  this entry's urgency drops to 'hardening'". Answer: the real
  quickshell 0.3.1 teardown (driving Noctalia 4.7.7) **does** commit
  after role destroy, and it kills the shell 1/1 in the real world.
  Urgency is P0, not hardening.

- Wire excerpt (`WAYLAND_DEBUG=1`): lock path is textbook —
  `bind(ext_session_lock_manager_v1)` → `lock()` → `get_lock_surface(#
  90)` → server `configure(4645, 1600, 900)` → client
  `ack_configure(4645)` → `locked()`. At unlock:
  `unlock_and_destroy()` → `ext_session_lock_surface_v1#90.destroy()`
  → `wl_surface#89.attach(nil)` → `wl_surface#89.commit()` → client
  prints `The Wayland connection broke. Did the Wayland compositor
  die?` and exits; screenshot goes to the all-black 30046-byte frame.
- Server side (the #34 disconnect logging earns its keep):
  `wayland client killed by a protocol error … error=ProtocolError {
  code: 0, object_id: 90, object_interface:
  "ext_session_lock_surface_v1", message: "Committed before the first
  ack_configure." }`, plus `session_lock: unlocking the session`
  immediately before — the unlock itself proceeds; only the client dies.
- Root cause, verified against the pinned Smithay source
  (`wayland/session_lock/surface.rs` in the cargo git checkout):
  `LockSurface::destroyed` calls `attributes.reset()`, which sets
  `last_acked = None`; the trailing null-commit then runs
  `pre_commit_hook`, sees `None`, and posts `CommitBeforeFirstAck`.
  This is exactly the #34 pattern (destruction handler resets role
  state to default; the next commit trips validation on that default),
  one role over: layer-shell got the post-destroy neutralization,
  session-lock didn't.
- Not a client bug: `destroy` "informs the compositor that the lock
  surface object will no longer be used" — nothing forbids the trailing
  null-commit on the now-unroled surface, and Smithay's own comment
  says the error exists to catch "attach a buffer without acking the
  initial configure", i.e. a first-commit-with-content, not teardown.
  The commit should be treated as the surface going unmapped.
- Note the DMS contrast: DMS's four kills (incl. its unlock teardown)
  all showed the *layer* signature (stale frame-callback `done`), so
  DMS's lock UI tore down layer surfaces; Noctalia locks through
  `ext-session-lock-v1` and dies on the *lock-role* signature instead.
  Whether DMS's unlock path also trips this second bug post-#34 is
  untested — don't assume.
- Rough size: S–M; see the existing entry for the fix analysis
  (why the #34 neutralize does not transfer, candidate resolutions).
  This probe adds: the fatal sequence is what stock quickshell sends
  (no minimal client needed to motivate it, though one still suffices
  to repro), and the kill is deterministic end-to-end (lock → PAM auth
  OK → `unlock_and_destroy` → destroy + null-commit → dead), not just
  in-harness. If the entry is updated, its resolution-(a) question can
  be closed as "confirmed fatal" and the entry raised to P0.

### 2. No `wlr-foreign-toplevel-management` (nor any `ext-` successor) — reproduces

- Zero `foreign_toplevel`/`toplevel_manager` wire traffic; global
  absent from `wayland-info`. Client-side proof: with a real `foot`
  window mapped (flexwm `msg windows` lists it, and it renders), the
  Noctalia launcher still shows Applications only — "6 results", no
  Windows/running-apps section. Same entry as DMS gap 2:
  `docs/backlog/protocols/foreign-toplevel-management.md`.

### 3. No `ext-idle-notify-v1` and no `idle-inhibit` — reproduces, verbatim

- Zero idle-protocol wire traffic. Noctalia logs the same sentence
  quickshell emits everywhere: `Cannot create idle monitor as
  ext-idle-notify-v1 is not supported by the current compositor.`
  (`IdleService`/`IdleInhibitor` start, then never fire.) Same entry:
  `docs/backlog/protocols/ext-idle-notify.md`.

### 4. No `wlr-output-management-unstable-v1` — reproduces

- Zero `output_management` wire traffic. Noctalia has a Settings →
  Display page (rendered, in the sidebar) but nothing to bind —
  display/scaling management is dead the same way DMS's was. Check for
  an `ext-` successor first, per the standing rule.

### 5. `xdg_popup` never configured — reproduces (untested live, same standing as DMS)

- Zero `xdg_popup` wire traffic across the whole session. Noctalia routes
  its panels through layer-shell like DMS does, so everything
  exercised works — but context menus/tooltips on the true-popup path
  have nowhere to go. Same entry:
  `docs/backlog/protocols/xdg-popup-input.md`. (The mapping half — no
  initial configure, so no popup maps — resolved 2026-09-14; see
  `docs/backlog/resolved/xdg-popup-initial-configure-resolved.md`.)

### 6. No screencopy / image-capture — reproduces

- Zero `screencopy`/`image_capture` traffic; neither global
  advertised. Noctalia's launcher/overview thumbnail views have nothing
  to consume. (flexwm agent screenshots go over flexwm IPC, so this is
  purely a shell-client gap, same as DMS gap 6.)

### 7. Compositor recognition — NOT a gap for Noctalia (biggest delta in the shell's favor)

- DMS gap 7 ("No compositor detected", workspace features dead) does
  not reproduce. Noctalia ships a generic backend:
  `ExtWorkspaceService: Service started (generic ext-workspace-v1)` /
  `CompositorService: Using generic ext-workspace backend (no
  recognized compositor env)`, and it works end-to-end — workspace
  group, output enter, workspace `1` with active state, live pill in
  the bar. There is nothing for flexwm to implement and nothing to ask
  upstream: consuming `ext-workspace-v1` for unknown compositors is
  exactly what the DMS report wished for. (Two harmless QML warnings
  ride along: `onWindowsetsChanged`/`onWindowsetProjectionsChanged`
  handlers with no matching signal in `ExtWorkspaceService.qml` —
  upstream cosmetic.)

### 8. Nice-to-have / cosmetic — same as DMS

- `ext-background-effect-v1`: `Cannot enable background effect as
  ext-background-effect-v1 is not supported`, graceful degradation, no
  blur. `wp_cursor_shape`, `single-pixel-buffer`, `presentation-time`,
  `relative-pointer`: nothing needed them. `xdg-activation-v1`:
  launching apps from the shell wasn't exercised end-to-end (apps were
  spawned via flexwm IPC instead); expect no focus handoff. Same
  entries as DMS gap 8.
- Gamma/night-light: `nightLight toggle` produced no gamma wire
  traffic and no pixel change — consistent with the DMS finding
  (protocol works, headless/nested has no LUT by design;
  `gamma_control.rs` stores the ramp for `--tty`). Noctalia-side
  status: untested beyond the toggle; no contrary evidence.

## Not gaps (VM environment, not flexwm)

PipeWire (`Failed to connect pipewire context`), UPower/PowerProfiles,
BlueZ, `dgop`-style helpers, GitHub version API (`Moved Permanently`
— no network path), wallpaper directory scan (`Scan failed for
headless`) — all session-bus/environment absences, same category as
the DMS report's list. The shell renders and functions around all of
them.

## Anything Noctalia needs that DMS didn't

Almost nothing — and what differs favors Noctalia:

- Same quickshell (0.3.1), same layer-shell version bound (v4 of
  advertised v5), same shm/software rendering path (MESA/`dri2`/`ZINK`
  warnings in the VM are noise; both shells render). No new protocol
  surface, no version-skew struggle: `nixpkgs#noctalia-shell` just runs.
- Noctalia needs *less* from flexwm than DMS, because of the generic
  ext-workspace backend (gap 7 above). It also documents its IPC
  (`ipc show`: 31 targets incl. `launcher`, `lockScreen`,
  `sessionMenu`, `notifications`, `wallpaper`, `nightLight`), which made
  this probe drivable without pointer-guessing; DMS needed its own
  `dms ipc` equivalents.
- One targeting note: the in-shell changelog says v4.7.7 (2026-05-13)
  "will be the last release of noctalia-shell v4. We will purely focus
  on Noctalia v5 from now on." If flexwm invests in shell-specific
  accommodations, aim them at whatever v5 speaks — or better, at the
  standard protocols both shells already share, so no per-shell work is
  needed at all.

## Recommended build order (Noctalia-aware)

1. **Gap 1 (lock-teardown kill)** — P0; the filed
   `session-lock-post-destroy-commit` item, now field-confirmed with a
   real quickshell client. Until it lands, nobody can lock the screen
   under either Quickshell shell without losing the session.
2. **Gap 5 (`xdg_popup`)** — already filed; unlocks menus/tooltips.
3. **Gap 3 (idle)** — already roadmap-next; unlocks auto-lock, pairing
   with the now-almost-proven session lock.
4. **Gap 2 (foreign-toplevel)** — already filed; unlocks window lists
   (Noctalia's launcher shows the hole clearly).
5. **Gap 4 (output-management)** — display settings.
6. **Gap 6 (screencopy)** — thumbnails/overview previews.
7. Nothing for gap 7 — Noctalia already does the right thing.

## Recommendation: DMS vs Noctalia vs own-overlay

**Noctalia is the better daily-drive shell target for flexwm today** —
with one blocking asterisk both shells share. It renders fully, its
launcher/settings/calendar/notifications all work, real D-Bus
notifications arrive, its IPC is richer and self-documenting, and —
decisively — its generic `ext-workspace-v1` backend gives live
workspace integration where DMS shows nothing. The protocol shopping
list is otherwise identical (idle, foreign-toplevel, output-mgmt,
popups, screencopy, blur), so nothing already learned is lost. The
asterisk: gap 1 means locking kills the shell 1/1, and a shell you
can't lock isn't daily-drivable — but the kill is now diagnosed to the
exact Smithay lines, same bug class as the already-fixed #34, so it
should be a small follow-up, not a research project. Own-overlay
remains the long-term independence play, but for *validating flexwm
against a real shell right now*, Noctalia gives more signal per gap
fixed. Fix gap 1 first either way — it unblocks both shells' clients.
