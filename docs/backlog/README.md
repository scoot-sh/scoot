# Backlog

Everything not yet scheduled or done, split by area. Each entry is one file
with YAML frontmatter (`title`, `status`, `area`, `priority`, `blocked`), so
the set is filterable without parsing prose:

```sh
rg -l 'status: "open"' docs/backlog                  # everything live
rg -l 'priority: "high"' docs/backlog                # what to pick next
rg 'blocked: ' docs/backlog/**/*.md | rg -v ': null' # what is waiting on something
```

`ROADMAP.md` is the index and carries the ordered work; this directory is
the detail. Prose is the original backlog entry, moved verbatim.

**Resolved entries** (struck through in the old monolith, kept for the
diagnosis history) live in [`resolved/`](./resolved/). They are archived, not
actionable.

## High priority

- [**`ext-idle-notify-v1` + `idle-inhibit-unstable-v1`**](./resolved/ext-idle-notify-resolved.md)
  — RESOLVED 2026-09-15: swayidle-style auto-lock trigger, field-proven live.
- [**Custom/client cursor support**](./resolved/cursor-theme-name-done.md)
  — RESOLVED 2026-09-15: real xcursor theme loading, which this entry had
  long (and wrongly) recorded as blocked on a license-clean asset. Shipping
  one is blocked; reading the user's own never was.
- [**The four protocols `foot` warned about**](./resolved/foot-protocol-warnings-done.md)
  — RESOLVED 2026-09-15 (issue #40, PR #41): `wp-cursor-shape-v1` (with ten
  procedurally-drawn shapes, so naming one gets you that shape),
  `xdg-activation-v1`, `xdg-toplevel-icon-v1` (name exposed as
  `WindowSnapshot::icon`), `text-input-v3` + `input-method-v2`. Verified by
  before/after on `foot`'s own warnings.
- [**`xdg-activation-v1` had no input-serial gate**](./resolved/activation-serial-validation-done.md)
  — RESOLVED 2026-09-16 (PR #42), found by independent review of the
  four-protocols PR: an unfocused client with no user interaction at all
  could self-activate its own surface. A token now has to name a real,
  recent key/button event actually delivered to the requesting client.
- [**Popup input: grabs, keyboard focus, layer-parented popups**](./resolved/xdg-popup-input-resolved.md)
  — RESOLVED 2026-09-16 (PR #44): `xdg_popup.grab` is honoured, with a
  stated focus precedence (lock > exclusive layer surface > popup grab >
  window). Locking dismisses any open popup grab and refuses new ones, so a
  menu left open at lock time cannot receive the password.

## Open

### Protocols
- [Foreign-toplevel management](./resolved/foreign-toplevel-list-done.md)
  — RESOLVED 2026-09-16 (PR #47): `ext-foreign-toplevel-list-v1`, the `ext-`
  successor the entry told its reader to check for, so a taskbar or switcher
  can list windows. Enumeration only (the protocol has no control requests);
  the identifier carries the `flexwm msg windows` id, which is the bridge to
  acting on a window over IPC. Quickshell binds the wlr protocol and ignores
  this one (measured), which is the entry below.
- [Quickshell's window list needs `wlr-foreign-toplevel-management-v1`](./resolved/wlr-foreign-toplevel-management-done.md)
  — RESOLVED 2026-09-16 (PR #50): the older protocol implemented alongside
  the `ext-` one, from the same window-lifecycle events. Enumeration
  (title/app id/output), `activate`, `close`, and `activated` as the one
  state bit flexwm can honestly answer; minimize/maximize/fullscreen are
  accepted and ignored, since the core has no concept of any of them.
  Verified live against the real quickshell: window list, click-to-focus and
  close all work.
- [`xdg-activation-v1` / IPC focus leaves the keyboard on a clicked layer surface](./resolved/activation-clicked-layer-keyboard-done.md)
  — RESOLVED 2026-09-16: `request_activation` and every focus-family IPC
  action now spend the click (`clicked_layer = None`), the way PR #50's own
  `activate` and a window click already did. A launcher or panel that stays
  mapped no longer keeps every keystroke after handing focus away.
- [`ext_workspace.rs`'s cross-client check takes two backend locks per manager, per `wl_output` bind](./protocols/ext-workspace-client-lookup-per-bind.md)
  — pre-existing, same review; `ObjectId::same_client_as` answers the exact
  same question without locking
- [Output management for shell display pages](./resolved/output-management-read-only-done.md)
  — RESOLVED 2026-09-16 (PR #49), read half only:
  `wlr-output-management-unstable-v1` (no `ext-` successor exists at the
  pinned rev, and Smithay has no helper for either, so the handler layer is
  hand-written against the generated wlr bindings the way `gamma_control.rs`
  is). One head for flexwm's one output, read from the same `Output` that
  configures `wl_output`.
- [`wlr-output-management` reconfiguration: `apply`/`test` always fail](./protocols/output-management-reconfiguration.md)
  — the deliberately-deferred other half of the entry above; gated on
  multi-output support, since nothing a configuration asks for exists yet
- [An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred](./protocols/wl-output-preferred-flag-on-late-mode.md)
  — pre-existing, found reviewing the entry above's own (correct) handling of the same event
- [Screen capture for clients (`wlr-screencopy` / `ext-image-copy-capture`)](./resolved/screencopy-capture-done.md)
  — HALF-RESOLVED 2026-09-16: `ext-image-copy-capture-v1` +
  `ext-image-capture-source-v1` for **output** capture, so `grim` and a
  workspace-overview preview work. `wlr-screencopy` deliberately not
  implemented alongside it — measured, both clients that matter speak the
  `ext-` protocol. flexwm's own IPC screenshot is unchanged.
- [Screen capture, toplevel half](./protocols/screencopy-toplevel-capture.md)
  — the per-window source a launcher's thumbnails need; needs a second render
  target per session, so split out rather than half-implemented
- [Screen capture: the session count is unbounded](./protocols/screencopy-session-cap.md)
  — found building the output half; why a cap was not simply added
- [Screen capture forces `Xrgb8888`'s undefined fourth byte opaque](./protocols/screencopy-xrgb-alpha-forcing.md)
  — found measuring a review finding on the output half: the forcing is ~13%
  of a release capture and ~77% of a debug one, to set a byte the format says
  is undefined and `grim` demonstrably ignores
- [`xdg-toplevel-icon-v1` pixel-buffer icons are not exposed](./protocols/toplevel-icon-buffers.md) — only the icon name reaches IPC
- [An IME popup over a lock screen is tracked but never drawn](./protocols/ime-popup-over-lock-screen.md)
- [An IME keyboard grab makes the activation gate credit a client that received nothing](./protocols/interaction-serial-ime-grab.md)
- [`spawn` hands its child no `XDG_ACTIVATION_TOKEN`](./protocols/activation-token-for-spawned-children.md)
- [Popup input](./resolved/xdg-popup-input-resolved.md)
  — RESOLVED 2026-09-16: `xdg_popup.grab` is honoured, with a stated focus
  precedence (lock > `exclusive` layer surface > popup grab > window /
  click-focused layer surface). Layer-parented popups turned out already to
  work; implementing the handler this entry asked for would have
  double-tracked them. Follow-up: the grab serial is still unvalidated.
- [`xdg_popup.grab` accepts any serial](./protocols/popup-grab-serial-validation.md) — any client can take the keyboard with no user action
- [An `exclusive` layer surface's own popup grab dismisses itself](./protocols/popup-grab-exclusive-self-dismiss.md)
- [A window-focus change doesn't dismiss an active popup grab](./protocols/popup-grab-survives-window-focus-change.md) — `windows`' `focused` can diverge from where keys go; relevant to computer-use targeting fidelity
- [An IME keyboard grab blocks every popup grab](./protocols/popup-grab-blocked-by-ime-grab.md) — no context menu opens anywhere while an IME holds the seat
- [Layer surface with no buffer still holds its exclusive zone](./protocols/layer-surface-bufferless-exclusive-zone.md)
- [Unbounded `ext_workspace_manager_v1` binds per client](./protocols/ext-workspace-object-binding-cap.md)
- [Lock surfaces are per-output; flexwm has one output](./protocols/lock-surfaces-per-output.md)
- [Unbounded lock surfaces via duplicate `wl_output` binds](./protocols/lock-surface-duplicate-wl-output.md)
- [Lock blanks immediately instead of waiting for the first surface](./protocols/session-lock-blank-timing.md)
- [`locked` sent on rendered frame, not confirmed vblank](./protocols/session-lock-vblank-confirm.md)
- [Lock manager global offered to every client](./protocols/session-lock-global-restriction.md)
- [First click on a fresh lock screen reaches nobody](./protocols/session-lock-first-click.md)
- [Smaller/general protocol gaps (bundled)](./protocols/protocol-gaps-general.md)
- [Niche protocol gaps (bundled)](./protocols/protocol-gaps-niche.md)

### IPC / computer use
- [Targeted input injection without moving seat focus](./ipc/targeted-input-injection.md) — the computer-use gap (research)
- [IPC bundle: usable rect, focus-workspace-index, ambient locked](./resolved/protocol-bundle-resolved.md) — RESOLVED 2026-09-15, no bump needed
- [No cap on concurrent IPC connections, and a half-closed client leaks one](./resolved/ipc-connection-cap-resolved.md) — RESOLVED 2026-09-16: 64 connections, refused with a reason past that, and a write-stall deadline that drops a peer which has stopped reading
- [Screenshot capture and encode run on the event-loop thread](./ipc/screenshot-encode-on-event-loop.md) — split out of the entry above; the ~12ms stall itself, which no bound closes
- [The accept loop swallows `EMFILE` and can spin the event loop](./ipc/accept-loop-swallows-emfile.md) — pre-existing, found reviewing the cap
- [The connection cap turns one client's leak into everyone's refusal](./ipc/connection-cap-denies-the-same-user.md) — accepted tradeoff, recorded so a future "the bar cannot connect" has somewhere to land
- [A large `msg type` blocks the event loop](./ipc/msg-type-blocks-event-loop.md)

### Input
- [`msg key` hard-codes `_L` modifier keysyms](./input/msg-key-modifier-resolution.md)
- [`msg type`: dead keys, compose, inactive layouts](./input/msg-type-dead-keys-compose.md)

### --tty / backend
- [Config-file key for the DRM device](./tty/tty-gpu-config-key.md) — **gated on Asahi confirmation**
- [Background color not painted where no window covers](./resolved/tty-background-not-painted-done.md)
  — RESOLVED 2026-09-16: it always was painted. The smoke test's background
  sample pixel sat on the cursor, which only `--tty` draws. Closes the
  duplicate `testing/` entry that had the right diagnosis all along.
- [Same-VT no-op VT-switch warning](./tty/vt-switch-same-vt-warning.md)
- [`O_CLOEXEC` request is a no-op at the libseat layer](./tty/tty-o-cloexec-noop.md) (informational)
- [`--tty` quit sometimes logs a DRM "restore previous state" EPERM](./tty/drm-teardown-restore-eperm.md)

### Core / config / rendering
- [`--width`/`--height` are unbounded `i32`s](./core/width-height-unbounded.md)
- [`[binds]` capital letter parses but never fires](./config/binds-capital-letter.md)
- [Per-frame `Vec` alloc in the cursor fallback path](./rendering/cursor-element-per-frame-alloc.md)
- [Cursor frames still tick while VT-paused](./rendering/cursor-frame-callback-when-paused.md)
- [`wl_surface.offset` doesn't move the cursor hotspot](./rendering/cursor-surface-offset-hotspot.md)
- [The pointer starts at the output's origin, not centred](./rendering/pointer-starts-at-origin.md)

### Security
- [Unbounded total shm reservation per client](./security/shm-total-per-client-unbounded.md)

### Packaging / tooling
- [Nix `src = self` invalidates the build on doc-only edits](./packaging/nix-src-fileset.md)
- [`x86_64-darwin` in `systems` breaks `flake check --all-systems`](./packaging/flake-x86-darwin-system.md)
- [`flake.nix` description drift](./packaging/flake-description-drift.md) (nit)
- [`smoke-test.sh` hardcodes temp paths](./testing/smoke-test-hardcoded-temp-paths.md)
- [`smoke-test.sh` background check samples the cursor under `--tty`](./resolved/tty-background-not-painted-done.md)
  — RESOLVED 2026-09-16, together with the `--tty` entry above that had
  independently (and wrongly) filed the same failure as a compositor bug.
- [Enhanced hardware/DRM testing ideas](./testing/hardware-testing-ideas.md) (research)
- [Extract a shared test harness; split the largest test files; adopt `cargo-nextest`](./resolved/large-test-file-organization-done.md)
  — RESOLVED 2026-09-16: `compositor/test_support.rs` now carries the
  real-client harness the five largest suites each reimplemented; the two
  biggest are split by concern; `cargo-nextest` is on the dev VM and in the
  documented verification set. Same tests, same assertions, 707 fewer lines
  of duplication.

### Meta
- [Rename `flexwm` → `flex`, split out `flexctl`](./meta/rename-flex-family.md) — decided, do this **last** in the burn-down (`flexbar` stays separate, undecided)
