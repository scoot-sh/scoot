---
title: "DMS (DankMaterialShell) enablement gaps — probe results 2026-09-14."
status: "open"
area: "protocols"
priority: "high"
blocked: null
---

# DMS (DankMaterialShell) enablement gaps — probe results 2026-09-14.

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
- Rough size: M. See
  `docs/backlog/protocols/foreign-toplevel-management.md` (check for an
  `ext-` successor first, per the standing rule).

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
  first, per the standing rule).

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
  remains is the input half, filed as
  `docs/backlog/protocols/xdg-popup-input.md` (grabs, keyboard focus,
  layer-parented popups).

### 6. No screencopy / image-capture (`wlr-screencopy`, `ext-image-capture-source`)

- DMS feature affected: launcher window thumbnails (`TileItem.qml`
  `ScreencopyView`) and the workspace overview live preview
  (`OverviewWindow.qml` `ScreencopyView`). Neither global is advertised.
  (flexwm screenshots for agents go over flexwm IPC, so this is purely a
  DMS-client gap.)
- Rough size: M. No existing backlog entry — file one when this is
  scheduled.

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

## Recommended build order

1. **Gap 1 (destroy kill)** — P0; the shell cannot survive normal use, and
   any Qt layer-shell client likely trips the same path.
2. **Gap 5 (`xdg_popup`)** — input half filed
   (`xdg-popup-input.md`; the mapping half resolved); unlocks
   menus/tooltips once the shell survives.
3. **Gap 3 (idle)** — already roadmap-next; unlocks auto-lock, pairing with
   the now-proven session lock.
4. **Gap 2 (foreign-toplevel)** — already filed; unlocks window lists.
5. **Gap 4 (output-management)** — display settings.
6. **Gap 6 (screencopy)** — thumbnails/overview previews.
7. **Gap 7 (DMS recognition)** — upstream issue; raises the ceiling from
   "reduced features" once 1–6 land.
