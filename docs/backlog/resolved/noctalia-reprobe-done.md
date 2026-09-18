---
title: "Noctalia enablement probe — results 2026-09-14, re-probed 2026-09-18 (gap 1 closed, ticket resolved)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Noctalia enablement probe — results 2026-09-14, re-probed 2026-09-18.

**RESOLVED 2026-09-18 (re-probe, no build).** The P0 gap 1 is closed in the
field: two full lock → PAM auth → `unlock_and_destroy` cycles on current
`main`, shell pid unchanged, live screenshots after, zero kill-signature
lines in server, shell or wire logs — the exact fatal teardown sequence is
on the wire and the connection survives it. Every other gap re-checks as
resolved, upstream, or deliberate (delta table below); nothing new was
found broken, so no follow-up tickets were filed and this entry moves to
`resolved/`. Detail is the re-probe section at the end; the original
2026-09-14 report is left intact below it.

Probe (not a build): does Noctalia (the other Quickshell-based desktop
shell — bar, notifications, launcher, lock screen, wallpaper; upstream
`noctalia-dev/noctalia-shell`, GPL-3.0 — nothing copied, so the license
only matters for future asset/code borrowing) fare better than DMS under
flexwm? Baseline: the DMS probe report
(`docs/backlog/resolved/dms-reprobe-done.md`, 2026-09-14) plus PR #34
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
  Windows/running-apps section. Same entry as DMS gap 2. **RESOLVED
  2026-09-16, in two halves.** flexwm implemented the `ext-` successor
  first (`docs/backlog/resolved/foreign-toplevel-list-done.md`) — but that
  is *not* what Quickshell binds. Measured on the same quickshell 0.3.1
  this probe used: the global was offered and never bound, and a minimal
  `ToplevelManager` config reported `count = 0` with a real window open.
  What this gap actually needed was
  `docs/backlog/resolved/wlr-foreign-toplevel-management-done.md` (PR #50),
  which the same probe now measures as listing the window with its title
  and app id, focusing it on a panel click, and closing it on request.

### 3. No `ext-idle-notify-v1` and no `idle-inhibit` — reproduces, verbatim

- Zero idle-protocol wire traffic. Noctalia logs the same sentence
  quickshell emits everywhere: `Cannot create idle monitor as
  ext-idle-notify-v1 is not supported by the current compositor.`
  (`IdleService`/`IdleInhibitor` start, then never fire.) Resolved
  2026-09-15: `docs/backlog/resolved/ext-idle-notify-resolved.md` --
  that sentence should now be gone on a current build.

### 4. No `wlr-output-management-unstable-v1` — reproduces

- Zero `output_management` wire traffic. Noctalia has a Settings →
  Display page (rendered, in the sidebar) but nothing to bind —
  display/scaling management is dead the same way DMS's was. Check for
  an `ext-` successor first, per the standing rule.
- Half-resolved 2026-09-16 (PR #49): there is no `ext-` successor at the
  pinned rev, and the query half now exists, so the Display page has
  something to bind and read.
  `docs/backlog/resolved/output-management-read-only-done.md`. Changing the
  mode/position/scale from that page is still refused, deliberately:
  `docs/backlog/resolved/output-management-reconfiguration-done.md`.

### 5. `xdg_popup` never configured — reproduces (untested live, same standing as DMS)

- Zero `xdg_popup` wire traffic across the whole session. Noctalia routes
  its panels through layer-shell like DMS does, so everything
  exercised works — but context menus/tooltips on the true-popup path
  have nowhere to go. Same entry:
  `docs/backlog/resolved/xdg-popup-input-resolved.md` (resolved
  2026-09-16 — grabs are honoured and layer-parented popups were already
  working). (The mapping half — no initial configure, so no popup maps —
  resolved 2026-09-14; see
  `docs/backlog/resolved/xdg-popup-initial-configure-resolved.md`.)

### 6. No screencopy / image-capture — reproduces

- Zero `screencopy`/`image_capture` traffic; neither global
  advertised. Noctalia's launcher/overview thumbnail views have nothing
  to consume. (flexwm agent screenshots go over flexwm IPC, so this is
  purely a shell-client gap, same as DMS gap 6.)
- **HALF-RESOLVED 2026-09-16 (PR #52).** `ext-image-copy-capture-v1` with
  `ext-image-capture-source-v1` is advertised, so the **overview preview**
  (an output source) has something to consume; the **launcher window
  thumbnails** need a per-window source and still do not — and the toplevel
  half was **CLOSED UNREACHABLE 2026-09-17 without building it** (stock
  quickshell routes a `Toplevel` source only to
  `hyprland-toplevel-export-v1`; see
  [`resolved/screencopy-toplevel-capture-done.md`](../resolved/screencopy-toplevel-capture-done.md)).
  Fallback:
  [`screencopy-shell-thumbnails-fallback.md`](../resolved/screencopy-shell-thumbnails-fallback-done.md)
  — CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build).
  See [`resolved/screencopy-capture-done.md`](../resolved/screencopy-capture-done.md).

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
4. **Gap 2 (foreign-toplevel)** — DONE 2026-09-16, both halves. The `ext-`
   one alone did *not* unlock this (quickshell binds the wlr protocol,
   measured); `resolved/wlr-foreign-toplevel-management-done.md` did.
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

## Re-probe 2026-09-18 (current `main` — gap 1 closed, ticket resolved)

Probe (not a build): re-drive Noctalia against current `main` per this
ticket's build order — gap 1 first — using the same drill as "How this
was measured" above. Verdict: **the shell survives everything, including
two full lock → PAM auth → `unlock_and_destroy` cycles, and every gap
2–8 re-checks as resolved, upstream, or deliberate. Nothing new is
broken; no follow-up tickets filed.** No compositor code changed in this
pass (pure probe + docs), so there is no cargo test/clippy/fmt delta to
report and `scripts/smoke-test.sh` was not re-run — the probe below is
the evidence.

### Setup (same drill, current revisions)

- flexwm at `87aa4fd` (current `main`), rebuilt in the dev VM
  (`/var/cargo-target/debug/flexwm`, timestamp Sep 18 02:24 UTC:
  `cargo build --manifest-path /mnt/flexwm/Cargo.toml --bin flexwm`
  from the 9p mount printed a real `Compiling flexwm` line and
  `Finished in 9.39s` — not the sub-2s ghost build).
- `flexwm --headless --width 1600 --height 900 --socket
  /run/user/1000/flexwm-noctalia.sock` (Wayland `wayland-1`), pid
  1972865, server log `/tmp/noc-reprobe-server.log`.
- Noctalia from nixpkgs, ephemeral only, no VM mutation:
  `nix shell nixpkgs#noctalia-shell nixpkgs#quickshell -c
  noctalia-shell` — noctalia-shell 4.7.7, noctalia-qs 0.0.12 (same
  versions as the 09-14 probe, so no client-version skew),
  `QT_QPA_PLATFORM=wayland`.
- Driven via `flexwm msg` (`screenshot`, `type`, `key`, `pointer
  move/click`; `FLEXWM_SOCKET` set since `--socket` names only the IPC
  socket) and `noctalia-shell ipc call <target> <fn>`. One full session
  under `WAYLAND_DEBUG=1` (`/tmp/noc-reprobe-wire.log`, 27,016 lines).
- Screenshots: local `/tmp/noc-reprobe-*.png` on the Mac during the
  probe, viewed, not committed. VM left clean (probe processes killed,
  temp sockets/logs/screenshots and `~/.config/noctalia` removed).
- Shell pid for the whole session: **1973057**, from 02:28 UTC (second
  start) through 02:37+ UTC across every cycle below.

### First-launch observation (no ticket filed)

The first shell start mapped **zero** layer surfaces in ~4 minutes:
process alive but idle in poll (`wchan poll_schedule_timeout`, ~0%
CPU), `~/.config/noctalia/settings.json` generated, ext-workspace
group/output/workspace-`1`/active events consumed per the qslog, no
error anywhere, and zero layer-surface lines in the server log — the
client never attempted to map, so there is nothing compositor-side to
blame. Kill + restart rendered in ≤20s (705509-byte frame: bar, clock,
tray, workspace pill `1`, owl wallpaper). Same nixpkgs revisions both
times. This leans shell-side (first-run init not completing until a
restart) and the shell runs fine afterwards, so it is recorded here,
not filed — a flexwm bug report on this evidence would be speculation.

### Primary question: gap 1 closed, twice

- Cycle 1: `ipc call lockScreen lock` → server `locking the session`
  (02:29:35) → lock screen renders ("Welcome back, flexwm developer!",
  password field focused, session buttons; 526430-byte shot) →
  `msg type "dev"` + `msg key Return` → shell log
  `pam.subprocess: Authenticated successfully` → server `unlocking the
  session` (02:29:54) → pid 1973057 unchanged → post-unlock shot
  705515 bytes: live desktop (bar + wallpaper + pill), not the
  30046-byte all-black kill frame.
- Cycle 2 (after the full sweep below, with `foot` mapped): lock →
  auth → unlock (`locking` 02:37:35, `unlocking` 02:37:40), pid
  unchanged, 426639-byte live shot (desktop + foot).
- Kill-signature grep across all three logs (server, shell stderr,
  wire): **0** lines matching `CommitBeforeFirstAck`, `killed by a
  protocol error`, or `The Wayland connection broke`.
- The wire carries the exact fatal sequence from the 09-14 report:
  `ext_session_lock_v1#36.unlock_and_destroy()` then
  `ext_session_lock_surface_v1#85.destroy()` →
  `wl_surface#82.attach(nil, 0, 0)` → `wl_surface#82.commit()` — and
  the connection survives it (traffic continues through cycle 2).
- The five `discarded [unknown]` lines are the known-benign startup
  frame-callback discards noted in the original report, without any
  `delete_id` cascade or connection break.

### Regression sweep (pid 1973057 throughout)

Tally 12 dismissals + 1 Escape (the original 14 included 2 first-run
modal pointer dismissals; no modals exist once the config is written,
so there is no counterpart this run): 3 launcher toggle-close cycles,
1 launcher open + `msg type "foot"` live-filter (search box narrows to
Foot / Foot Client / Foot Server — keyboard focus reaches the
overlay; 526591-byte shot) + `msg key Escape` dismiss, 2 calendar
toggles, 2 control-center toggles, 3 settings toggles — one left open
for the 553187-byte rendered shot (General→Basics, full sidebar: Bar,
Dock, Lock Screen, Session Menu, Idle, Audio, Display…).

- Bar + wallpaper + workspace pill render in every shot; clock
  advances normally (02:28 → 02:36 across the session).
- Real notification delivery: nixpkgs-libnotify `notify-send "Hello
  from Noctalia re-probe"` renders a toast top-right (705217-byte
  shot shows the full text).
- `foot` coexistence: `msg action spawn foot` maps, tiles (id 1,
  `12,42 782x846`, focused) and renders beside the shell
  (426719-byte shot).

### Delta check on gaps 2–8

- **Gap 2 (foreign-toplevel) — resolved, verified live.** The shell
  binds `zwlr_foreign_toplevel_manager_v1` at startup (`bind(8, …)` at
  02:28:31, right after the ext-workspace bind) and the foot toplevel
  arrives complete: `toplevel(new id …)` → `title("")`/`app_id("")` →
  `output_enter` → `state` → `done`, then `app_id("foot")` +
  `title("foot")` with `done`. `ext_foreign_toplevel_list_v1` is
  offered and never bound — re-confirms PR #47's measurement on this
  exact client. Launcher UI shows Applications only in the views
  checked; two pointer clicks on the monitor-icon tab did not switch
  tabs, so a running-windows section was not verified visually, and
  panel-click focus / close were not re-driven here (both proven live
  in PR #50's own probe). No new ticket.
- **Gap 3 (idle) — resolved.** The `Cannot create idle monitor as
  ext-idle-notify-v1 is not supported` sentence is absent from both
  shell stderr and the qslog; `IdleService` + `IdleInhibitor` log
  `Service started`; the shell binds `ext_idle_notifier_v1`
  (`bind(32, …)` at 02:28:32). Monitors actually firing on real idle
  was not re-driven — the swayidle field proof stands in
  `resolved/ext-idle-notify-resolved.md`. No new ticket.
- **Gap 4 (output-management) — no new ticket.** The Display page was
  opened by pointer (first click landed on Region — pointer input
  reaches the settings list fine — second click on Display at 425,645
  landed): Brightness / Night Light tabs, output read as `headless
  (1600x900 @ 1x)` — from `wl_output`, because the shell **never
  binds** `zwlr_output_manager_v1` (zero `bind(28` on the whole wire
  log). The query half exists for clients that want it; the
  deliberate reconfiguration refusal is already its own item. No new
  ticket.
- **Gap 5 (xdg_popup) — same standing, no new ticket.** Zero
  `xdg_popup` wire traffic across the session; grabs are implemented
  but this client routes everything through layer surfaces and never
  exercises them.
- **Gap 6 (screencopy) — no new ticket; gate is shell-side.** Both
  capture managers (globals 9/10) plus `zwp_linux_dmabuf_v1` (11) are
  advertised, but the shell never binds 9 or 10: the workspace-pill
  click opened no overview and no `ScreencopyView` was ever
  instantiated, so no client-side gate (dmabuf readiness or otherwise)
  was even reached. PR #60 already proved real pixels flow to a
  quickshell `ScreencopyView` over shm on headless. The toplevel half
  stays CLOSED UNREACHABLE and the thumbnail fallback NEEDS-UPSTREAM
  per the resolved records; current Noctalia has no per-window
  live-thumbnail view at all.
- **Gap 7 — still not a gap.** Generic `ext-workspace-v1` backend,
  live pill `1`, qslog shows group creation, output added, workspace
  `1` with active state.
- **Gap 8 (cosmetic) — unchanged/deliberate.** `Cannot enable
  background effect as ext-background-effect-v1 is not supported`
  still logged (graceful, no blur protocol — deliberate);
  `wp_cursor_shape_manager_v1` bound at startup;
  `wp_presentation` bound only on Qt's internal EGL render
  connections, never the shell's main connection;
  relative-pointer, single-pixel-buffer and gamma-control unbound;
  `xdg-activation-v1` bound but launching apps from the shell (focus
  handoff) still unexercised, same as 09-14.

### What closes, what got filed, next build item

Closes: this ticket (gap 1 field-proven fixed; everything else
resolved/upstream/deliberate). Filed: nothing — no new breakage
found. The first-launch no-surfaces observation is recorded above,
not filed (client-idle, zero server-side evidence). Next build item
is the orchestrator's pick from the backlog — nothing in this probe
blocks or redirects it.
