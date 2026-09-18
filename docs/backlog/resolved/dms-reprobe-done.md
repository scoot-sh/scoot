---
title: "DMS (DankMaterialShell) enablement gaps — probe results 2026-09-14, re-probed 2026-09-18 (gap 1 closed, ticket resolved)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# DMS (DankMaterialShell) enablement gaps — probe results 2026-09-14, re-probed 2026-09-18.

**RESOLVED 2026-09-18 (re-probe, no build).** The P0 gap 1 stays closed
in the field: the exact spotlight teardown plus two full lock → PAM auth
→ `unlock_and_destroy` cycles on current `main`, shell pid unchanged
throughout, live screenshots after every step, zero kill-signature lines
in server, shell or wire logs — both fatal sequences (layer destroy +
null-commit, lock-role destroy + null-commit) are on the wire with the
connection surviving. DMS's unlock path does **not** trip the lock-role
signature Noctalia died on. Every other gap re-checks as resolved,
upstream, deliberate, or shell-side presentation; gap 8's unexercised
launch question is answered (DMS-spawned app maps focused). Nothing new
was found broken, so no follow-up tickets were filed and this entry
moves to `resolved/`. Detail is the re-probe section at the end; the
original 2026-09-14 report (plus its own evening re-probe) is left
intact below it.

Probe (not a build): can DMS, the Quickshell-based desktop shell (bar,
notifications, launcher, lock screen, clipboard history), run under flexwm
today, and what exactly is missing? Result: **it runs, renders, and even
locks/unlocks — but dismissing any overlay deterministically kills the whole
shell** (gap 1, 4/4 repro, wire evidence below). Everything else is a ranked
list behind that.

## How this was measured

- flexwm at `56b3ebe` (current `main`, post-#32), rebuilt in the dev VM
  (`/var/cargo-target/debug/flexwm`, built 2026-09-14 21:42 UTC from the
  9p mount — `cargo build` exits clean, target dir is guest-local).
- `flexwm --headless --width 1600 --height 900 --socket
  /run/user/1000/flexwm-dms.sock` (`WAYLAND_DISPLAY=wayland-1`).
- DMS from nixpkgs, ephemeral only, no VM mutation:
  `nix shell nixpkgs#dms-shell nixpkgs#quickshell -c dms run`
  (dms-shell 1.5.3, quickshell 0.3.1, `QT_QPA_PLATFORM=wayland`).
- Driven via `flexwm msg` (`screenshot`, `type`, `key`, `pointer
  move/click`) and `dms ipc call <module> toggle`; screenshots pulled to the
  Mac and viewed. Advertised globals cross-checked with `wayland-info`
  (18 globals — see gap list for what is absent).
- Screenshots: local `/tmp/dms-*.png` on the Mac during the probe, not
  committed; every claim below cites the command plus the log signature so
  it can be re-derived live.

## What runs today

| DMS module | Verdict | Evidence |
| --- | --- | --- |
| Bar + wallpaper (layer-shell Top + Background) | works | `dms:bar`, `quickshell` background surfaces mapped and rendered (screenshot: clock, tray icons, wallpaper) |
| Launcher/spotlight overlay | works until dismissed | `SPOTLIGHT_TOGGLE_SUCCESS`; typing `foot` via `msg type` filters the list live; `Escape` then kills the shell (gap 1) |
| Notification center + real delivery | works | `notify-send` (nixpkgs libnotify) arrives over D-Bus and renders ("Hello from probe", `Current (1)`) |
| Clipboard history overlay | works until dismissed | renders ("No recent clipboard entries found"); same dismissal kill (gap 1) |
| Lock screen (`ext-session-lock-v1`) | works incl. unlock auth | lock surface renders fullscreen; typing the dev password + `Return` → quickshell PAM `Authenticated successfully`, daemon `Screen lock active changed: false`; then the unlock teardown kills the shell (gap 1) |
| Gamma / night mode (`zwlr_gamma_control_manager_v1`) | protocol works, no pixels headless | daemon `Wayland gamma control initialized successfully`; direct daemon-API calls `wayland.gamma.setEnabled` → `{"success":true}`, `wayland.gamma.setTemperature {"temp": 3500}` → `{"success":true}`; day-vs-night screenshots identical, which is **by design** (`gamma_control.rs`: headless/nested has no LUT, ramp stored, applied on `--tty` only) |
| DankDash overlay, notification-center popout (layer-shell, keyboard-interactive) | works | clock click → full dashboard (calendar, weather, sliders); bell click → `dms:notification-center-popout` anchored top-right |
| Pointer input to layer surfaces | works | wire log shows `wl_pointer.enter` + button press/release reaching the bar surface |

## Gap list

### 1. Dismissing any overlay layer surface breaks the client's Wayland connection (P0 — nothing else matters until this is fixed)

- DMS feature affected: every overlay — launcher, notifications modal,
  clipboard history, and the post-unlock lock-surface teardown. 4/4 kills.
- Missing behavior: none missing — this is a **bug**, not a missing
  protocol. After the client sends `zwlr_layer_surface_v1.destroy` +
  `wl_surface.attach(nil)` + `commit`, the compositor emits
  `wl_callback.done` (event 0) for an object id the client no longer knows
  (`discarded [unknown]#36.[event 0]`), then Qt drops the connection
  (`The Wayland connection broke. Did the Wayland compositor die?`,
  quickshell exit 255, all shell surfaces gone, screenshot goes to the
  deterministic all-black 30046-byte frame). flexwm's own log shows
  **nothing** at info or debug level — no error, no disconnect.
- Wire excerpt (`WAYLAND_DEBUG=1`, `dms-run2.log`):
  `-> zwlr_layer_surface_v1#48.destroy()`,
  `-> wl_surface#42.attach(nil, 0, 0)`, `-> wl_surface#42.commit()`,
  then `discarded [unknown]#36.[event 0]` followed by the server's
  `delete_id(43)`, `delete_id(36)`, `done` for #43, `delete_id(48)`.
  Object #36 was the last frame callback on the destroyed surface.
  Same signature on all four kills (spotlight, notifications, clipboard,
  unlock); `xdg-toplevel` close (`msg action close` on foot ×3) does **not**
  trigger it, so it is specific to the layer-surface/frame-callback destroy
  path — likely frame callbacks firing against a surface being torn down
  (headless frame timer vs. destroy dispatch ordering), in flexwm's
  layer-shell handling or the pinned Smithay core.
- Rough size: M. Isolate first (minimal layer-shell client: map, request
  frames, destroy mid-flight — no DMS needed), fix the ordering, add a
  regression test that destroys a layer surface with a frame callback
  in flight, and consider logging client disconnects/errors so the next
  such kill leaves a trace.

### 2. No `wlr-foreign-toplevel-management` (nor any `ext-` successor)

- DMS feature affected: launcher "running windows" search, dock/taskbar
  window lists, `FocusedApp` widget, `ProcessListModal` window actions.
  DMS uses Quickshell's generic `ToplevelManager` (29 use sites in the
  1.5.3 QML) — with no compositor detected it has nothing to list.
- Log evidence: none needed beyond `wayland-info` (global absent) plus the
  launcher screenshots showing Applications/Settings only, no Windows
  section.
- Rough size: M. **RESOLVED 2026-09-16, in two halves.** The `ext-`
  successor exists and is implemented
  (`docs/backlog/resolved/foreign-toplevel-list-done.md`), which is what the
  standing rule asked for — but it is not what Quickshell binds. Measured
  against quickshell 0.3.1 (this probe's own build): flexwm offered
  `ext_foreign_toplevel_list_v1` and quickshell never bound it, so a minimal
  `ToplevelManager` reported `count = 0` with a real window open. The wlr
  protocol then landed alongside it
  (`docs/backlog/resolved/wlr-foreign-toplevel-management-done.md`, PR #50)
  and the same probe now reports the window, its title and its app id; a
  click on a real Quickshell `PanelWindow` focuses the named window, and
  `close` closes it. `ProcessListModal`'s minimise/maximise actions stay
  inert — flexwm's core has no concept of either.

### 3. No `ext-idle-notify-v1` and no `idle-inhibit`

- DMS feature affected: idle/auto-lock, auto-suspend, auto-dim. DMS
  `IdleService.qml` arms five Quickshell `IdleMonitor`s, which need
  `ext-idle-notify-v1` or `org_kde_kwin_idle` — neither is advertised, so
  they never fire. Manual lock (gap: none) works; *automatic* lock is dead.
- Rough size: M. Resolved 2026-09-15:
  `docs/backlog/resolved/ext-idle-notify-resolved.md` (field-proven
  with real swayidle: idle → resume → re-idle). DMS's `IdleService`
  monitors should now fire; auto-lock is a daemon config away.

### 4. No `wlr-output-management-unstable-v1`

- DMS feature affected: Settings → display/output management and
  interface scaling. The Go daemon reports `WlrOutput management
  initialized successfully` and then `Received empty outputs list` (twice),
  and `wl_output` itself is well-formed (1600x900@60 current+preferred) —
  the empty list is specifically the output-management path with no global
  to bind. DMS docs list this protocol as required for generic
  compositors.
- Rough size: M–L (new protocol surface; check for an `ext-` successor
  first, per the standing rule). Half-resolved 2026-09-16 (PR #49): there is
  no `ext-` successor at the pinned rev, so the wlr protocol it is, and the
  query half DMS's daemon is asking for now exists — `Received empty outputs
  list` should be gone on a current build.
  `docs/backlog/resolved/output-management-read-only-done.md`.
  Reconfiguration (the "Settings → display" *writes*, and interface scaling)
  is still refused, deliberately:
  `docs/backlog/resolved/output-management-reconfiguration-done.md`.

### 5. `xdg_popup` never configured

- DMS feature affected: context menus (`FileBrowserItemContextMenu`,
  `ProcessContextMenu`, launcher `SectionHeader` menus) and tooltips
  (`DankTooltipV2`). DMS routes its *own* popups through extra layer-shell
  surfaces (measured: zero `xdg_popup` wire traffic in a full session, and
  the notification popout maps fine as `dms:notification-center-popout`),
  so the shell reads as working — but the true-popup paths have nowhere to
  go. Dynamic tooltip check (hover bell 4s) was negative but weak (may be
  DMS config); the backlog entry's "no popups map at all" stands.
- Rough size: M. The mapping half (no initial configure, so no popup
  maps) resolved 2026-09-14:
  `docs/backlog/resolved/xdg-popup-initial-configure-resolved.md`. What
  remains was the input half (grabs, keyboard focus, layer-parented
  popups), resolved 2026-09-16:
  `docs/backlog/resolved/xdg-popup-input-resolved.md`.

### 6. No screencopy / image-capture (`wlr-screencopy`, `ext-image-capture-source`)

- DMS feature affected: launcher window thumbnails (`TileItem.qml`
  `ScreencopyView`) and the workspace overview live preview
  (`OverviewWindow.qml` `ScreencopyView`). Neither global is advertised.
  (flexwm screenshots for agents go over flexwm IPC, so this is purely a
  DMS-client gap.)
- Rough size: M. No existing backlog entry — file one when this is
  scheduled.
- **HALF-RESOLVED 2026-09-16 (PR #52).** `ext-image-copy-capture-v1` with
  `ext-image-capture-source-v1` is advertised, so the **overview live
  preview** (an output source) has something to consume. The **launcher
  window thumbnails** are a per-window source and still do not — and the
  toplevel half was **CLOSED UNREACHABLE 2026-09-17 without building it**:
  stock quickshell 0.3.1 routes a `Toplevel` `ScreencopyView` source
  exclusively to `hyprland-toplevel-export-v1` (wire evidence + version-exact
  source in
  [`resolved/screencopy-toplevel-capture-done.md`](../resolved/screencopy-toplevel-capture-done.md)).
  The fallback (region crop out of the output) is
  [`screencopy-shell-thumbnails-fallback.md`](../resolved/screencopy-shell-thumbnails-fallback-done.md) —
  CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build): the overview
  preview lights up on shipped `main`, the crop recipe is proven live,
  and DMS's `TileItem.qml` hard-requires a `Toplevel` source, so the
  shells must change. Note its first measurement also covers why the overview preview still shows
  nothing in a quickshell shell despite PR #52 (the client's dmabuf
  readiness gate). See
  [`resolved/screencopy-capture-done.md`](../resolved/screencopy-capture-done.md).

### 7. DMS does not recognize flexwm ("No compositor detected") — mostly upstream work

- DMS feature affected: workspace indicator/switching and everything
  compositor-specific. `CompositorService: Unrecognized Wayland socket
  owner: flexwm - falling back to env detection` → `No compositor
  detected`. Note flexwm **has** `ext-workspace-v1`; DMS just never
  consults the generic protocol — it only speaks niri/Hyprland/Sway/Mango
  IPC. There is nothing for flexwm to implement here beyond what exists;
  the ask is DMS-upstream (recognize `flexwm`, preferably by consuming
  `ext-workspace-v1` for unknown compositors rather than another bespoke
  branch).
- Rough size: XS flexwm-side (docs/recognition only); the real work is a
  DMS upstream issue.

### 8. Nice-to-have / cosmetic

- `ext-background-effect-v1` (blur): DMS logs `Compositor does not
  support ext-background-effect-v1` and degrades gracefully (no blur).
  Low; no entry filed.
- `wp_cursor_shape_manager_v1`, `single-pixel-buffer-v1`,
  `presentation-time`, `relative-pointer-v1`: nothing in the probe needed
  them; existing entries under
  `docs/backlog/protocols/protocol-gaps-general.md` cover the general case.
- `xdg-activation-v1`: launching apps from DMS was not exercised
  end-to-end (no app launch attempted); expect launched windows to map
  without activation/focus handoff. Already filed as
  `docs/backlog/resolved/foot-protocol-warnings-done.md` (implemented
  2026-09-15; this probe's own question -- whether launching apps from DMS
  hands focus over correctly -- is still unexercised).

## Not gaps (VM environment, not flexwm)

NetworkManager, bluetooth/BlueZ, UPower/power-profiles, polkit agent,
PipeWire/PulseAudio, GeoClue, `dgop` — all absent from the VM's session
bus, all logged by DMS/quickshell as unavailable, none compositor-related.
The weather widget still showed 24°C and notifications/audio-adjacent UI
rendered; these only limit live data, not protocol conclusions.

## Re-probe 2026-09-14 (evening, post-#34 + post-#36) — gap 1 closed for DMS

- flexwm at `7f937bd` (current `main`), rebuilt in the dev VM
  (`/var/cargo-target/debug/flexwm`, incremental, 8s).
  DMS 1.5.3 + quickshell 0.3.1 from nixpkgs, ephemeral, no VM mutation.
- **Unlock path: fixed.** `dms ipc call lock lock` → lock screen renders
  (clock, password field focused); `msg type "dev"` + `msg key Return` →
  PAM auth OK → server log shows clean `locking the session` →
  `unlocking the session` with no protocol-error kill; dms/quickshell
  pids unchanged; post-unlock screenshot is the live desktop (bar +
  wallpaper, 70407 bytes — not the 30046-byte all-black kill frame).
  The "inferred fixed, proven only for Noctalia" caveat is closed.
- **Overlay dismissal: survives.** `dms ipc call spotlight toggle` →
  `SPOTLIGHT_TOGGLE_SUCCESS`, overlay renders (search box, real VM
  .desktop entries, Applications only — still no Windows section, gap 2
  stands); `msg key Escape` dismisses; shell survives. This is the exact
  spotlight teardown that killed DMS 4/4 in the original probe.
- Side notes: fresh DMS config shows a first-run wizard (xdg-toplevel
  "Welcome" window); pointer clicks reach it (dismissed via its ✕).
  `flexwm msg` needs `FLEXWM_SOCKET=<ipc socket>` — `--socket` sets the
  IPC socket, the Wayland display stays `wayland-1`.
- Screenshots: local `/tmp/dms-reprobe-*.png` on the Mac, not committed.

## Re-probe 2026-09-18 (current `main` — gap 1 stays closed, ticket resolved)

Probe (not a build): re-drive DMS against current `main` per this
ticket's remaining live questions — (a) gap 1 stays closed and DMS's
unlock path doesn't trip the lock-role signature Noctalia died on, (b)
delta-check gaps 2–8 against everything resolved since (incl. the 512
live-buffer bound from PR #94), (c) record gap 7's recognition status.
Verdict: **the shell survives everything, including the exact
spotlight teardown and two full lock → PAM auth →
`unlock_and_destroy` cycles, and every gap re-checks as resolved,
upstream, deliberate, or shell-side presentation. Nothing new is
broken; no follow-up tickets filed.** No compositor code changed in
this pass (pure probe + docs), so there is no cargo test/clippy/fmt
delta to report and `scripts/smoke-test.sh` was not re-run — the probe
below is the evidence.

### Setup (same drill, current revisions)

- flexwm at `96398c0` (current `main`, incl. PR #95), rebuilt in the
  dev VM (`/var/cargo-target/debug/flexwm`, timestamp Sep 18 02:46 UTC:
  an initial `cargo build --manifest-path /mnt/flexwm/Cargo.toml
  --bin flexwm` from the 9p mount ghost-built (`Finished in 0.74s`,
  no `Compiling` line), so `cargo clean -p flexwm && cargo build`
  before trusting it — real build `Compiling flexwm`, `Finished in
  32.16s`).
- `flexwm --headless --width 1600 --height 900 --socket
  /run/user/1000/flexwm-dms.sock` (Wayland `wayland-1`), server log
  `/tmp/dms-reprobe-server.log`.
- DMS from nixpkgs, ephemeral only, no VM mutation:
  `nix shell nixpkgs#dms-shell nixpkgs#quickshell -c dms run` —
  dms-shell 1.5.3, quickshell 0.3.1 (same quickshell as both prior
  probes, so no client-version skew), `QT_QPA_PLATFORM=wayland`.
  (`dms ipc` needs both packages on PATH — with `dms-shell` alone it
  fails `exec: "qs": executable file not found in $PATH`.)
- Driven via `flexwm msg` (`screenshot --out`, `type`, `key`,
  `action spawn`; `FLEXWM_SOCKET` set since `--socket` names only the
  IPC socket) and `dms ipc call <module> <fn>`. One full session under
  `WAYLAND_DEBUG=1` (`/tmp/dms-reprobe-wire.log`, 7,398 lines).
- Screenshots: local `/tmp/dms-reprobe-*.png` on the Mac during the
  probe, viewed, not committed. VM left clean (probe processes killed,
  temp sockets/logs/screenshots and the probe-created
  `~/.config/DankMaterialShell` removed).
- Shell pid for the whole session: **1974743**, from ~02:47 UTC through
  02:55+ UTC across every cycle below.

### Primary question (a): gap 1 stays closed, lock-role signature absent

- Overlay dismissal: `dms ipc call spotlight toggle` →
  `SPOTLIGHT_TOGGLE_SUCCESS`, overlay renders (search box, real VM
  .desktop entries, Applications 10, no Windows section; 127895-byte
  shot) → `msg type "foot"` live-filters 10 → 3 (Foot / Foot Client /
  Foot Server; 98393-byte shot — keyboard focus reaches the overlay)
  → `msg key Escape` dismisses → 70353-byte shot **byte-identical to
  the baseline** (`cmp`), pid 1974743 unchanged. This is the exact
  spotlight teardown that killed DMS 4/4 in the original probe.
- Lock → auth → unlock, twice: `dms ipc call lock lock` → server
  `locking the session` (02:49:02) → lock screen renders (clock,
  password field focused; 89105-byte shot) → `msg type "dev"` +
  `msg key Return` → shell log `pam.subprocess: Authenticated
  successfully` → server `unlocking the session` (02:49:18) → pid
  unchanged → 70366-byte live desktop (bar + wallpaper; 13 bytes off
  baseline — the clock advanced 2:48 → 2:49, so genuinely live, not a
  frozen frame). Cycle 2 (02:55:14, with two `foot` windows mapped):
  same path, pid unchanged, 50368-byte live shot.
- Kill-signature grep across all three logs (server, shell stderr,
  wire): **0** lines matching `killed by a protocol error`,
  `CommitBeforeFirstAck`, or `The Wayland connection broke`.
- The wire carries both fatal sequences with the connection
  surviving. Cycle 1: `ext_session_lock_v1#55.unlock_and_destroy()`
  then `ext_session_lock_surface_v1#57.destroy()` →
  `wl_surface#53.attach(nil, 0, 0)` → `wl_surface#53.commit()` —
  traffic continues after (`done(140148)`, `Screen lock active
  changed: false`). Cycle 2: `unlock_and_destroy()` (#68) →
  `ext_session_lock_surface_v1#41.destroy()` → `attach(nil)` →
  `commit()`. **DMS's unlock path does not trip the lock-role
  signature** — the fix holds for this client too.

### Regression sweep (pid 1974743 throughout)

- Notification delivery: nixpkgs-libnotify `notify-send "Hello from
  DMS re-probe"` renders a toast top-right (79718-byte shot); the
  notification-center modal (`notifications toggle` →
  `NOTIFICATION_MODAL_TOGGLE_SUCCESS`) shows Current (1) + History
  (2) with the full text (83660-byte shot); `notifications close` →
  success, shell survives.
- Clipboard overlay: `clipboard toggle` → `CLIPBOARD_TOGGLE_SUCCESS`,
  renders ("No recent clipboard entries found"; 75725-byte shot),
  Escape-dismissed, shell survives.
- DankDash: `dash toggle overview` → `DASH_TOGGLE_SUCCESS` (a bare
  `dash toggle` is refused — it takes a tab argument), renders fully
  (clock, weather, September 2026 calendar, sliders; 124280-byte
  shot); `dash close` → `DASH_CLOSE_SUCCESS`, shell survives. (A
  `dashboard toggle` target does not exist — `Target not found`;
  the target is `dash`.)
- `foot` coexistence: `msg action spawn foot` maps, tiles (id 1,
  `12,56 782x832`, focused) and renders beside the shell
  (55552-byte shot); the bar shows a `foot` task indicator.
- Second spotlight cycle with foot running: Applications 16, still no
  Windows section (see gap 2 below).

### Delta check on gaps 2–8 (question b) and 7 (question c)

- **Gap 2 (foreign-toplevel) — resolved, verified live.** The shell
  binds `zwlr_foreign_toplevel_manager_v1` at startup (02:47:05) and
  the foot toplevel arrives complete: `app_id("foot")` +
  `title("foot")` on the wire. `ext_foreign_toplevel_list_v1` is
  offered and never bound — re-confirms PR #47's measurement on this
  exact client. Launcher UI caveat (presentation, not protocol): with
  a real foot running, spotlight still shows Applications only, and
  searching "foot" shows Applications 3 + Settings 3 — no
  Windows/running-apps section. The compositor publishes the list
  (wire-proven); DMS's QML doesn't surface it in this view, and
  panel-click focus / close were already proven live against the real
  quickshell in PR #50's own probe. No new ticket.
- **Gap 3 (idle) — resolved.** The idle error sentence is absent from
  the shell log; `ext_idle_notifier_v1` (v2) is advertised. The shell
  never binds it on its main connection (no auto-lock config armed —
  monitors firing on real idle was not re-driven; the swayidle field
  proof stands in `resolved/ext-idle-notify-resolved.md`). No new
  ticket.
- **Gap 4 (output-management) — no new ticket.** The daemon reports
  `WlrOutput: found zwlr_output_manager_v1`,
  `wlr-output-management capability detected`, and `Validating
  profiles against current outputs` — `Received empty outputs list`
  is gone. The shell's main connection never binds the manager
  itself (reads `wl_output`, like Noctalia). The deliberate
  reconfiguration refusal is already its own item. No new ticket.
- **Gap 5 (xdg_popup) — same standing, no new ticket.** Zero
  `xdg_popup` wire traffic across the session; grabs are implemented
  but this client routes everything through layer surfaces and never
  exercises them.
- **Gap 6 (screencopy) — no new ticket; gate is shell-side.** Both
  capture managers (globals 9/10) plus `zwp_linux_dmabuf_v1` (11)
  are advertised, but the shell never binds 9 or 10; dmabuf is bound
  only by Qt's internal EGL connections, never for a
  `ScreencopyView`. The toplevel half stays CLOSED UNREACHABLE and
  the thumbnail fallback NEEDS-UPSTREAM per the resolved records. No
  new ticket.
- **Buffer cap (PR #94) — not tripped, no finding.** The server log
  has zero error/refusal lines of any kind across the whole session,
  and the shell lived throughout — a refusal would kill the client
  with a protocol error. Wire-log accounting (libwayland connections
  only; the Go daemon's own connections aren't `WAYLAND_DEBUG`
  visible): 19 buffers created, 13 destroyed, 6 live at copy time —
  two orders of magnitude below the 512 live-buffer bound. No new
  ticket.
- **Gap 7 (recognition) — still "No compositor detected", recorded,
  not filed upstream (out of scope).** Shell log still shows
  `CompositorService: Unrecognized Wayland socket owner: flexwm -
  falling back to env detection` → `No compositor detected`. Noted:
  the shell *binds* `ext_workspace_manager_v1` at startup but its
  CompositorService still doesn't take the generic path — upstream's
  call, same as 09-14.
- **Gap 8 — improved: the unexercised launch question is answered.**
  Pressing Return on the spotlight's Foot entry launches via DMS: a
  second foot maps (id 2), tiles in the next column, and
  **focused: true** — activation/focus handoff works end to end
  (the shell binds `xdg_activation_v1`; 50381-byte shot shows both
  terminals). Also bound live: `wp_cursor_shape_manager_v1`.
  Cosmetic remainder unchanged: `Cannot enable background effect as
  ext-background-effect-v1 is not supported` ×7 (graceful, no blur
  protocol — deliberate).

### What closes, what got filed, next build item

Closes: this ticket (gap 1 field-proven still closed, with the
lock-role signature explicitly checked; everything else
resolved/upstream/deliberate/shell-side, plus gap 8's launch question
now answered). Filed: nothing — no new breakage found. Not re-driven
here (stated, not papered over): idle monitors firing on real idle,
panel-click focus/close from DMS's own UI, overview-preview pixels
for DMS specifically, and the dmabuf-allocating-client half of PR
#60's matrix (environment limit, per that record). Next build item
is the orchestrator's pick from the backlog — nothing in this probe
blocks or redirects it.

## Recommended build order

1. **Gap 1 (destroy kill)** — P0; the shell cannot survive normal use, and
   any Qt layer-shell client likely trips the same path.
2. **Gap 5 (`xdg_popup`)** — both halves resolved
   (`resolved/xdg-popup-input-resolved.md`); unlocks menus/tooltips once
   the shell survives.
3. **Gap 3 (idle)** — already roadmap-next; unlocks auto-lock, pairing with
   the now-proven session lock.
4. **Gap 2 (foreign-toplevel)** — DONE 2026-09-16, both halves. The `ext-`
   one alone did *not* unlock this (quickshell binds the wlr protocol,
   measured); `resolved/wlr-foreign-toplevel-management-done.md` did.
5. **Gap 4 (output-management)** — display settings.
6. **Gap 6 (screencopy)** — thumbnails/overview previews.
7. **Gap 7 (DMS recognition)** — upstream issue; raises the ceiling from
   "reduced features" once 1–6 land.
