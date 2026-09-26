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
the detail. scootbg, the wallpaper daemon, keeps its own backlog in
[`docs/scootbg/backlog/`](../scootbg/backlog/README.md). Prose is the original backlog entry, moved verbatim.

**Resolved entries** (struck through in the old monolith, kept for the
diagnosis history) live in [`resolved/`](./resolved/). They are archived, not
actionable — and they still say `flexwm`, the name this project carried until
2026-09-18. That is deliberate: they record evidence (nix store paths, typed
input strings, screenshot paths, exact commands run) that renaming would
falsify. Read `flexwm` there as `scoot`.

## High priority

- [**The dmabuf advertisement killed every GL client**](./resolved/dmabuf-advertised-but-never-imported-done.md)
  — RESOLVED 2026-09-18, regression from `599be4e` found live: scoot
  advertised `zwp_linux_dmabuf_v1` but answered every import `failed`, which
  for `create_immed` is a fatal `InvalidWlBuffer`. Mesa took the advertised
  dmabuf path over `wl_shm` and died, so noctalia v5 could not start a
  session at all — a black screen with a cursor. Fixed by importing for
  real: Smithay's `PixmanRenderer` `mmap`s linear dmabufs on the CPU, so this
  needed no GPU and did not wait on roadmap item 6.
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
  the identifier carries the `scoot msg windows` id, which is the bridge to
  acting on a window over IPC. Quickshell binds the wlr protocol and ignores
  this one (measured), which is the entry below.
- [Quickshell's window list needs `wlr-foreign-toplevel-management-v1`](./resolved/wlr-foreign-toplevel-management-done.md)
  — RESOLVED 2026-09-16 (PR #50): the older protocol implemented alongside
  the `ext-` one, from the same window-lifecycle events. Enumeration
  (title/app id/output), `activate`, `close`, and `activated` as the one
  state bit scoot can honestly answer; minimize/maximize/fullscreen are
  accepted and ignored, since the core has no concept of any of them.
  Verified live against the real quickshell: window list, click-to-focus and
  close all work.
- [`xdg-activation-v1` / IPC focus leaves the keyboard on a clicked layer surface](./resolved/activation-clicked-layer-keyboard-done.md)
  — RESOLVED 2026-09-16 (PR #53): `request_activation` and every focus-family IPC
  action now spend the click (`clicked_layer = None`), the way PR #50's own
  `activate` and a window click already did. A launcher or panel that stays
  mapped no longer keeps every keystroke after handing focus away.
- [`ext_workspace.rs`'s cross-client check takes two backend locks per manager, per `wl_output` bind](./resolved/ext-workspace-client-lookup-per-bind-done.md)
  — RESOLVED (PR #68): `ObjectId::same_client_as` answers the exact same
  question without locking. Originally filed pre-existing, same review.
- [Flake: activation taskbar-click precondition misses under extreme parallel load](./resolved/activation-taskbar-click-settle-flake-done.md)
  — RESOLVED 2026-09-18 (test-only): verify-first found three distinct
  load-only trip mechanisms at the same test, not one (settle-insufficiency,
  a racing `activate` spending the click before the assert, and
  `focus_before` read after the racing dispatch) — settle-until-hittable on
  the click's own hit test plus synchronous press-asserts, production pin
  untouched
- [`ext-workspace-v1` workspace activation moves window focus but leaves the keyboard on a clicked layer surface](./resolved/ext-workspace-clicked-layer-keyboard-done.md)
  — RESOLVED 2026-09-16 (PR #54): confirmed real, not not-a-bug — two
  fail-first tests proved the switch path consults `clicked_layer`, and the
  fix spends the click before `act` (refusing a locked session first, and
  spending it on the already-active early return too, so the path agrees
  with IPC `FocusWorkspaceIndex` rather than differing by transport)
- [Output management for shell display pages](./resolved/output-management-read-only-done.md)
  — RESOLVED 2026-09-16 (PR #49), read half only:
  `wlr-output-management-unstable-v1` (no `ext-` successor exists at the
  pinned rev, and Smithay has no helper for either, so the handler layer is
  hand-written against the generated wlr bindings the way `gamma_control.rs`
  is). One head for scoot's one output, read from the same `Output` that
  configures `wl_output`.
- [`wlr-output-management` reconfiguration](./resolved/output-management-reconfiguration-done.md)
  — CLOSED 2026-09-18 as a deliberate refusal (accepted tradeoff, kept as a
  landing spot): verify-first found no Smithay helper at the pinned rev, no
  authorization concept in the protocol and no privilege model to attach one
  to, per-backend honesty gaps (nested is host-constrained, custom modes are
  unhonorable, disable has no state to land in), and zero write attempts from
  either probed shell. Revisit conditions are in the record.
- [An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred](./resolved/wl-output-preferred-flag-on-late-mode-done.md)
  — RESOLVED 2026-09-17: fixed as filed — `set_mode` now marks the new mode
  preferred *before* `change_current_state` sends it (no batching at the
  pinned rev, verified in source), one fix covering `--nested` and `--tty`,
  pinned by a fail-first harness test asserting both protocols agree
- [Screen capture for clients (`wlr-screencopy` / `ext-image-copy-capture`)](./resolved/screencopy-capture-done.md)
  — HALF-RESOLVED 2026-09-16: `ext-image-copy-capture-v1` +
  `ext-image-capture-source-v1` for **output** capture, so `grim` and a
  workspace-overview preview work. `wlr-screencopy` deliberately not
  implemented alongside it — measured, both clients that matter speak the
  `ext-` protocol. scoot's own IPC screenshot is unchanged.
- [Screen capture, toplevel half](./resolved/screencopy-toplevel-capture-done.md)
  — CLOSED UNREACHABLE 2026-09-17 by phase-1 probe (no build): stock
  quickshell 0.3.1 routes a `Toplevel` capture source exclusively to
  `hyprland-toplevel-export-v1`, so the ext toplevel-source manager would
  never be bound. Fallback filed as
  [shell thumbnails without a toplevel protocol](./resolved/screencopy-shell-thumbnails-fallback-done.md).
- [Minimal, honest `zwp_linux_dmabuf_v1`](./resolved/linux-dmabuf-advertisement-done.md)
  — RESOLVED 2026-09-17 (PR #60): the readiness-gate follow-up — default
  feedback with the real scanout `dev_t` plus the two LINEAR formats shm
  serves, imports answered `failed`; quickshell's overview preview displays
  over shm on headless, no-node, `--nested` and `--tty`.
- [Shell window thumbnails without a toplevel protocol](./resolved/screencopy-shell-thumbnails-fallback-done.md)
  — CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build): the overview
  preview lights up with real pixels on shipped `main`, and the
  screen-source + clip-crop recipe for per-window thumbnails is proven
  live pixel-for-pixel — but DMS's `TileItem.qml` hard-requires a
  `Toplevel` source and current Noctalia has no per-window
  live-thumbnail view, so the change belongs upstream. No compositor
  work follows; `hyprland-toplevel-export-v1` stays out of scope.
- [Screen capture: the session count is unbounded](./resolved/screencopy-session-cap-done.md)
  — RESOLVED (PR #70): frames capped, sessions decided. Found building the
  output half; the entry records why a cap was not simply added
- [Screen capture forces `Xrgb8888`'s undefined fourth byte opaque](./resolved/screencopy-xrgb-alpha-forcing-done.md)
  — RESOLVED (conditional, PR #71). Found measuring a review finding on the output half: the forcing is ~13%
  of a release capture and ~77% of a debug one, to set a byte the format says
  is undefined and `grim` demonstrably ignores
- [`xdg-toplevel-icon-v1` pixel-buffer icons are not exposed](./resolved/toplevel-icon-buffers-done.md)
  — RESOLVED 2026-09-18: name-only stands as the honest scope (verified, not
  assumed — no leak, nothing to release, both list protocols icon-less, no
  consumer for pixels); four harness tests pin the buffer half against the
  live-buffer budget.
- [An IME popup over a lock screen is tracked but never drawn](./resolved/ime-popup-over-lock-screen-done.md)
  — RESOLVED 2026-09-17: the locked path gathers each current lock surface's
  popup tree and sends it frame callbacks, with the trust decision recorded
  (an IME is trusted with pixels for the focused field's candidate window,
  which shows it nothing composition doesn't already route through it).
  Five harness tests: inclusion + caret placement + frames, background-xdg
  and background-IME exclusion, unlock restore over two cycles, disable
  dismiss.
- [An IME keyboard grab makes the activation gate credit a client that received nothing](./resolved/interaction-serial-ime-grab-done.md)
  — RESOLVED 2026-09-17 (decide + pin, no behavior change): crediting the
  focused window is the intended outcome, so the decision is recorded and
  pinned by two harness tests (real `grab_keyboard`, token creation plus
  redemption) rather than changed. The ticket's three options were each worse
  than the status quo.
- [`spawn` hands its child no `XDG_ACTIVATION_TOKEN`](./resolved/activation-token-for-spawned-children-done.md)
  — RESOLVED 2026-09-17 (PR #65): `State::spawn` mints via
  `create_external_token` and sets it on the child's `Command`, under both
  existing bounds (30s from spawn, one of the same 64 slots, swept the same
  way); a full table means no token, never an eviction. Pinned by seven
  harness tests around a real spawned child.
- [Popup input](./resolved/xdg-popup-input-resolved.md)
  — RESOLVED 2026-09-16: `xdg_popup.grab` is honoured, with a stated focus
  precedence (lock > `exclusive` layer surface > popup grab > window /
  click-focused layer surface). Layer-parented popups turned out already to
  work; implementing the handler this entry asked for would have
  double-tracked them. Follow-up: the grab serial is now validated (next
  entry).
- [`xdg_popup.grab` accepts any serial](./resolved/popup-grab-serial-validation-done.md)
  — RESOLVED 2026-09-16 (PR #56): the grab must name a real, recent
  key/button/`enter` event delivered to the grabbing client (or continue
  its own open menu); background clients can no longer take the keyboard
  unprompted. `start_drag` needs its own analysis and stays open separately.
- [An `exclusive` layer surface's own popup grab dismisses itself](./resolved/popup-grab-exclusive-self-dismiss-done.md)
  — RESOLVED 2026-09-17 (PR #72): grant and pre-emption both check the
  grab's root — an exclusive surface no longer outranks its own menu,
  while a different exclusive surface still does; rule 3 now true as written
- [A window-focus change doesn't dismiss an active popup grab](./resolved/popup-grab-focus-divergence-done.md)
  — RESOLVED 2026-09-16 (PR #55): grab semantics unchanged, but `msg windows`
  now reports `popup_grab` per window so an agent can tell focus from where
  keys actually go; relevant to computer-use targeting fidelity
- [An IME keyboard grab blocks every popup grab](./resolved/popup-grab-blocked-by-ime-grab-done.md)
  — RESOLVED 2026-09-18: verify-first found the reverse order unhandled (an
  IME grabbing while a menu held the keyboard left it mapped with no
  keyboard) — a live grab displaced by a foreign keyboard grab is now
  dismissed with `popup_done` on the next dispatch, so either order ends
  with no menu while the IME holds the seat. Precedence pinned
  (lock > exclusive layer > IME grab > popup grab) and documented in the
  README; lock and serial-gate interplay pinned alongside.
- [Layer surface with no buffer still holds its exclusive zone](./resolved/layer-surface-bufferless-exclusive-zone-done.md)
  — RESOLVED 2026-09-18 (decide + pin, no behavior change): the zone applies
  from the buffer-less initial commit the protocol mandates (so windows never
  jump when the first buffer lands), not the first buffer; re-verified in the
  pinned Smithay `arrange` that filtering would mean forking the geometry.
  Three new harness tests pin the edges (never-draws held until disconnect,
  buffer-less destroy, post-buffer zone drop), each confirmed fail-first by
  neutering; no timeout by design, lifetime bounded by disconnect/destroy.
- [Unbounded `ext_workspace_manager_v1` binds per client](./resolved/ext-workspace-object-binding-cap-done.md)
  — RESOLVED 2026-09-17 (PR #74): one shared `BindBudget` — 8 binds per
  client across all four globals (ext-workspace, ext/wlr toplevel lists,
  output management), refused with each global's own `finished`; includes
  re-homing the ext list off Smithay state and an idle-deferred refusal
  (backend panics on in-bind destructor events)
- [Lock surfaces are per-output; scoot has one output](./resolved/session-lock-per-output-done.md)
  — RESOLVED 2026-09-18 (PR #103, pin + document, no behavior change): the first
  blanked frame on the one output confirms the lock whatever the surface
  count (zero-surface half already pinned), every admitted surface shares
  that output's size (pinned by a new two-surface suite, first-created on
  top, keyboard on the first, resize reaching all), and the single-output
  marker's doc (now `Outputs::primary`) lists the four session-lock sites
  multi-output must revisit. The
  duplicate-bind admission question stays with its own open entry below.
- [Unbounded lock surfaces via duplicate `wl_output` binds](./resolved/session-lock-duplicate-output-done.md)
  — RESOLVED 2026-09-18 (refuse, no Smithay patch): a second live surface
  for an already-covered output is refused with the protocol's own
  `duplicate_output` error (code 3 on the lock), keyed on the physical
  `Output` Smithay's resource-identity guard admits past; destroying the
  surface frees the output for a rebuild. Neither probed shell (DMS,
  Noctalia) ever holds two at once. Supersedes PR #103's two-surface
  composition pins; the single-surface per-output pins stand.
- [Lock blanks immediately instead of waiting for the first surface](./resolved/session-lock-blank-timing-done.md)
  — RESOLVED 2026-09-18 (decide + pin, no behavior change): the accept →
  input-captured → first-frame-blanks → `locked`-after-blank ordering
  verified in-harness and pinned by a new combined test (fail-first
  proven by neutering the accept-time transition); the niri-shape wait
  stays declined, revisitable only if the black flash proves annoying in
  daily use
- [`locked` sent on rendered frame, not confirmed vblank](./resolved/session-lock-vblank-confirm-done.md)
  — RESOLVED 2026-09-17 (PR #84): `locked` waits for the vblank of the flip carrying
  the blanked frame under `--tty` (sequence-tracked; one-second fallback
  confirms anyway rather than hanging the locker); headless/nested confirm
  on render unchanged
- [A lock surface mapped after the confirming frame never appears on live `--tty`](./resolved/session-lock-surface-not-drawn-live-done.md)
  — CLOSED UNREPRODUCED 2026-09-17 by isolate-first probe (no PR): six
  live sessions all green with pixel censuses, and the suspected damage
  hole disconfirmed by mechanism audit (all surface damage flows through
  the generic commit handler + new-element rules). Likely artifact class:
  hundreds of stale probe sockets on the dev VM (since swept); future
  probes must pin `--socket` + `WAYLAND_DISPLAY`. Adjacent code-traced
  present-skip damage finding filed as
  [present-skip eats frame damage](./resolved/present-skip-eats-frame-damage-done.md).
- [A held pointer lock survives a session lock, so a client keeps pointer input while locked](./resolved/pointer-lock-session-lock-done.md)
  — RESOLVED: the lock transition deactivates the held constraint (game sees
  `unlocked`), focus lands on the lock surface, and unlock re-arms through
  the ordinary arrival path. Confinement needed no fix (fail-open +
  leave-deactivation already handled it; pinned).
- [Lock manager global offered to every client](./resolved/session-lock-global-restriction-done.md)
  — CLOSED 2026-09-18 as an accepted tradeoff (no code): the protocol
  defines no privilege, the pinned Smithay filter is hide-from-registry
  only, and no available primitive (peer creds, security-context, bind
  budget) keys a locker allow-list without breaking the ordinary-client
  lock flows both shells use; kept as the landing spot
- [First click on a fresh lock screen reaches nobody](./resolved/session-lock-first-click-done.md)
  — RESOLVED 2026-09-17 (PR #76): pointer focus is re-derived on the commit
  that maps the lock surface, so the first click lands on it without the
  mouse having to move; recognition is one branch plus one typemap probe,
  zero new state
- [Smaller/general protocol gaps (bundled)](./protocols/protocol-gaps-general.md)
- [Niche protocol gaps (bundled)](./protocols/protocol-gaps-niche.md)

### IPC / computer use
- [Targeted input injection without moving seat focus](./ipc/targeted-input-injection.md) — the computer-use gap (research)
- [IPC bundle: usable rect, focus-workspace-index, ambient locked](./resolved/protocol-bundle-resolved.md) — RESOLVED 2026-09-15, no bump needed
- [No cap on concurrent IPC connections, and a half-closed client leaks one](./resolved/ipc-connection-cap-resolved.md) — RESOLVED 2026-09-16: 64 connections, refused with a reason past that, and a write-stall deadline that drops a peer which has stopped reading
- [Screenshot capture and encode run on the event-loop thread](./resolved/screenshot-encode-off-thread-resolved.md) — RESOLVED 2026-09-17 (PR #57): PNG encode moved to a single FIFO worker; per-connection ordering via refused-with-retry, 4 captures max globally, `wait-idle` unchanged
- [The accept loop swallows `EMFILE` and can spin the event loop](./resolved/accept-loop-emfile-resolved.md) — RESOLVED 2026-09-17 (PR #66): pending connections shed on `EMFILE` instead of spinning the accept loop; originally pre-existing, found reviewing the cap
- [The connection cap turns one client's leak into everyone's refusal](./resolved/connection-cap-denies-the-same-user-done.md) — CLOSED 2026-09-17 as an accepted tradeoff (no workload has hit it; per-pid sub-cap stays unbuilt as speculative); kept as the landing spot for a future "the bar cannot connect"
- [A large `msg type` blocks the event loop](./resolved/msg-type-blocks-event-loop-resolved.md) — RESOLVED 2026-09-17: `type` text capped at 16,384 characters per request (sized by measurement; worst case ~75ms), refused naming the limit and the split-workaround, counted in characters not bytes
- [`scoot msg` panics when its stdout reader goes away](./resolved/msg-client-broken-pipe-done.md) — RESOLVED 2026-09-17 (PR #62): per-write EPIPE handling at every client-binary stdio site, quiet exit 0; compositor unaffected
- [IPC focus actions run a full `apply` even when nothing moves](./resolved/focus-action-no-op-fast-path-done.md) — RESOLVED 2026-09-17 (PR #67): already-there focus actions skip `act` and run only the keyboard half, mirroring PR #54; relative steps deliberately left on the full path

### Input
- [`msg key` hard-codes `_L` modifier keysyms](./resolved/msg-key-modifier-resolution-done.md) — RESOLVED 2026-09-17 (PR #83): modifiers resolve through the keymap probe like `type_text` (either hand's key; toggle-option layouts work); `msg key A` still refuses
- [`msg type`: dead keys, compose, inactive layouts](./resolved/msg-type-dead-keys-compose-done.md) — RESOLVED 2026-09-18 (PR #121): two-key dead-led sequences from the session-locale compose table (é on `de`, dead-ASCII gaps closed — all 95 ASCII on all fourteen swept Latin layouts); inactive groups, lock/latch levels and `Multi_key` 3-key sequences stay refused
- [`zwp_tablet_manager_v2` (drawing-tablet input)](./resolved/tablet-v2-done.md)
- [[nested] pointer clicks never reach layer-shell surfaces](./resolved/layer-shell-pointer-clicks-done.md) — CLOSED 2026-09-21 as could-not-reproduce (gh #182): the issue's exact `--nested` probes deliver every click shape on current `main` and on the reported rev alike; kept as a wire-level pinning test, no compositor change. Shared root with #183 refuted for the input path.

### --tty / backend
- [Config-file key for the DRM device](./resolved/tty-gpu-config-key-done.md)
  — RESOLVED 2026-09-18: `[tty] gpu` names the device `--gpu` would
  (exactly that device, no fallback, fail-closed startup error; `--gpu`
  wins when both name one), verified live on the dev VM single-GPU
  (byte-identical default path, fail-closed refusal, flag precedence).
  The residual it carried -- confirming the Apple Silicon case on the
  reporter's own hardware -- is CLOSED 2026-09-18: the automatic search
  works there unattended, so `--gpu` is a convenience on that machine and
  not a requirement.
- [Background color not painted where no window covers](./resolved/tty-background-not-painted-done.md)
  — RESOLVED 2026-09-16: it always was painted. The smoke test's background
  sample pixel sat on the cursor, which only `--tty` draws. Closes the
  duplicate `testing/` entry that had the right diagnosis all along.
- [Same-VT no-op VT-switch warning](./resolved/vt-switch-same-vt-warning-done.md)
  — RESOLVED 2026-09-18: `change_vt` compares the request against the
  kernel's live displayed VT (`/sys/class/tty/tty0/active`, no new `Tty`
  field) before calling libseat, answering a new quiet `IgnoredSameVt`
  (plain IPC `Ok`) instead of the hedged `Requested` warning; unknown
  display falls back to asking. Real switch-away/back (pause/reactivate +
  modeset + byte-identical repaint) verified live, 5b unregressed.
- [`O_CLOEXEC` request is a no-op at the libseat layer](./resolved/tty-o-cloexec-noop-done.md)
  — RESOLVED 2026-09-18: the flag was dead (`LibSeatSession::open` takes
  `_flags` at the pinned rev, re-verified in source) and is removed; the
  guarantee it appeared to give still holds via libseat's
  `MSG_CMSG_CLOEXEC` receive, now pinned by a real-spawn test that also
  proves the bit load-bearing (a marker without it IS inherited — kept as
  the test's permanent positive control). Per-source audit filed under
  Security.
- [`--tty` quit sometimes logs a DRM "restore previous state" EPERM](./resolved/drm-teardown-restore-eperm-done.md)
  — RESOLVED 2026-09-18: strace-proven our own teardown racing itself (the
  restore-on-drop lives behind an `Arc` cloned into the event loop's DRM
  notifier, so it runs after the seat socket closes); `Tty` now pauses the
  device in its own `Drop`. Fail-first live 6/6 → 0/6.
- [`--tty` hotplug can only switch to a connector its current CRTC can drive](./resolved/tty-connector-switch-crtc-done.md)
  — RESOLVED 2026-09-18: `retarget` falls through to a CRTC switch
  (build-first-swap-on-success, no `Option<DrmSurface>` refactor — the
  ticket's anticipated refactor proved unnecessary once the per-CRTC plane
  claims were re-verified in source); total failure stays put and retries.
  Gamma LUT length re-read per CRTC, live control failed only on change.
  Four fail-first harness tests; the switch itself unverified live
  (single-CRTC dev VM), legacy blind-probe limit stated in the record.
- [`--tty` hotplug follow-up: confirm the two unreproduced paths on real hardware](./core/tty-hotplug-confirmation.md) — gh #48 stays open: new-mode-list on the same connector (vkms `edid_override` tried 2026-09-26, inert on the dev VM's 6.18.50 — still needs vfkit/rescale-capable hardware), and fallback to a *different* connector, with before/after proof. Since multi-output phase E the fallback applies only when no driven connector is left; with another screen lit an unplug removes an output instead (confirmed by a physical DP-1 replug on the Asahi M2 Air, 2026-09-25). The Hold/Reconnected halves of the neighbouring [multi-output remainder](./core/multi-output-remainder.md) follow-up 1 were proven live on vkms the same day.

### Core / config / rendering
- [`scoot --version`](./resolved/cli-version-flag-done.md) — RESOLVED 2026-09-21: `scoot --version` and `scootctl --version` print `scoot <version> (ipc protocol <N>)` from one shared helper (no drift, no session needed); the bare `version` word stays the remote IPC request by deliberate spelling decision.
- [[nested] top-layer content missing from frames with no toplevels](./resolved/layer-content-without-toplevels-done.md) — CLOSED 2026-09-21 as could-not-reproduce (gh #183): a real Top-layer bar with a committed buffer draws with zero toplevels on current `main` and on the reported rev alike (live `--nested` matrix + harness pixel pins); the field symptom is the designed bufferless-bar shape (zone reserved, nothing painted), so the client had almost certainly committed no buffer — no compositor change.- [`--width`/`--height` are unbounded `i32`s](./resolved/width-height-bounded-done.md)
  — RESOLVED 2026-09-17: refused past 65535 per axis at parse (the most DRM
  itself can report for a mode axis); `Rect::inset`/`right()`/`bottom()`,
  `scroll_into_view` and the arrange on-screen test saturate.
- [`[binds]` capital letter parses but never fires](./resolved/binds-capital-letter-done.md)
  — RESOLVED 2026-09-17: single ASCII letters fold to lowercase at
  config-parse time with a warning (`"A"` means plain `a`, not `shift+a`);
  `msg key A` still refuses.
- [Per-frame `Vec` alloc in the cursor fallback path](./resolved/cursor-element-per-frame-alloc-done.md)
  — CLOSED DELIBERATE with measured numbers: the fallback `vec![...]` is
  already the minimal allocation (448 bytes, capacity exactly 1, ~107ns net
  per call), firing 0 times/sec at idle and at most ~62.5/sec during dirty
  `--tty` frames; the ticket's push-into-a-local-`Vec` shape allocates 4x
  the bytes (1792, measured), and a persistent buffer's ripple costs more
  than ~7µs/s saves. Pinned by capacity tests.
- [Flake: parked-captures poll sees an extra `frame_serial` advance under full-suite load](./resolved/screencopy-parked-poll-flake-done.md)
  — RESOLVED 2026-09-18 (test-only): the filed mechanism was corrected
  (`delivered`-lag at park time, measured at an unmoving serial — not an
  advance between park and poll); quiescence wait plus synchronize-then-assert
  retry, production pin untouched
- [A `present()` skipped for an in-flight flip consumes that frame's damage](./resolved/present-skip-eats-frame-damage-done.md)
  — RESOLVED 2026-09-18 (PR #107): the filed in-flight shape self-heals
  (Smithay extends empty damage with history); the real loss was the
  refused commit/page-flip arm (freed slot kept a fresh age, retry read
  `None`, no vblank owed) — fixed with `note_write_failed` (free slot +
  clear age, per-slot) and a bounded frame-timer retry (3 consecutive
  refusals, then quiet).
- [A screencopy frame parked for a lock's blank is never re-armed after the vblank confirm](./resolved/screencopy-parked-across-lock-confirm-done.md)
  — RESOLVED 2026-09-17 (PR #93): `confirm_lock` itself owns one
  `ensure_ticking()` (structural — future confirm paths inherit it);
  parked frames deliver post-confirm, pixel-verified blank.
- [Cursor frames while VT-paused](./resolved/cursor-frame-callback-when-paused-done.md)
  — RESOLVED 2026-09-17: `render()` (and the frame-callback dispatch) is
  skipped while the `--tty` session holds no DRM master, so a paused
  session neither renders frames `present()` would drop nor wakes clients
  to paint them; reactivation still mode-sets and repaints fully.
- [`wl_surface.offset` moves the cursor hotspot](./resolved/cursor-surface-offset-hotspot-done.md)
  — RESOLVED 2026-09-18: PR #106's NEEDS-UPSTREAM triage re-derived and
  overturned (Smithay's own anvil does the decrement compositor-side, so
  scoot can too). `Cursor::note_surface_commit` decrements the hotspot by
  this commit's `buffer_delta` (saturating — both operands are
  client-controlled `i32`), gated on the active cursor surface; four
  fail-first harness tests.
- [The pointer starts at the output's origin, not centred](./resolved/pointer-starts-at-origin-done.md)
  — RESOLVED 2026-09-17: centred once at startup through
  `pointer_move_quietly` (no idle-timer reset, no focus, no interaction
  serial); resize and VT-switch reactivation leave it where the user left
  it; the smoke test's park-the-pointer workaround stays as harmless
  protection.

### Security
- [Live `wl_shm` pools per client](./resolved/shm-pool-count-cap-done.md) — RESOLVED 2026-09-17: at most 128 live pools per Wayland client (refused with `InvalidStride`, released on destroy/disconnect); the byte total stays open behind an upstream size accessor (proven unknowable at the pinned rev). **But see the entry below: the fd/mapping bound it documents is not the bound it has.**
- [The live-pool cap does not bound fds or mappings, which is what its docs claim](./resolved/shm-pool-cap-misses-retained-fds-done.md)
  — RESOLVED 2026-09-17: claims corrected everywhere (pool count bounds
  live objects + the address-space envelope, not fds/mappings), and the
  real fix landed as a per-client live-`wl_buffer` cap (512, refused with
  a protocol error per creating interface, released on destroy/disconnect)
  that catches exactly the bypass shape. The open ticket's
  "upstream-gated" expectation proved wrong -- buffers are fully
  observable, unlike pool internals.
- [No cap on Wayland connection count](./resolved/wayland-connection-cap-done.md) — RESOLVED 2026-09-18: the EMFILE-on-accept kill is fixed (Wayland listener sheds like the IPC one; pre-fix binary proven dead live, post-fix alive), and the count itself is closed as an accepted tradeoff (any usable count admits the two greedy connections that fill the table, so a count denies shells while stopping nothing)
- [A global fd/buffer ceiling across Wayland connections](./resolved/wayland-global-fd-ceiling-done.md) — RESOLVED 2026-09-18: a compositor-wide pressure ceiling with a shed strategy — newcomers shed past 128 free fds (Wayland EOF, IPC refused with a reason naming the pressure), past-grace creations (128 buffers / 64 pools) refused with the interfaces' own protocol errors, under-grace clients never refused for another's greed; sized from live measurement (idle 14, +foot 17), proven live with a 374-connection horde plus shed/survive/recover
- [Every compositor fd must carry close-on-exec](./resolved/spawn-fd-cloexec-audit-done.md) — RESOLVED 2026-09-18 (audit + test-only pin, no production change): every fd source verified at creation — event-loop, channel, listener/spare/accepted-socket, seatd, shm/dmabuf receipt, sealed memfds, libinput/udev, transient file reads — measured live on headless and `--tty` (every non-stdio fd carries the bit; an IPC-spawned child inherits none) with pinned sources re-verified where libraries create the fd; the spawn pin now also covers socket, `try_clone` and eventfd markers, each proven sensitive by neutering
- [Pin the fd-pressure grace conjunction's boundaries](./resolved/fd-pressure-grace-boundary-pins-done.md) — RESOLVED 2026-09-18 (pin, no behavior change): `pressure_refusal`'s pure conjunction split out as `pressure_refusal_for` and pinned at both operators and both graces (129 refused, 128 passes; either half alone passes); the ceiling record's 400 stands as "two at grace", the permitted maximum is 404

### Packaging / tooling
- [Nix package can reach neither GPU tier](./resolved/nix-gpu-tiers-done.md)
  — RESOLVED 2026-09-21 (gh #177): `packages.scoot` force-links libEGL
  (niri's `--no-as-needed` trick, Linux-only) so packaged `--renderer gles`
  reaches the OS's EGL drivers (dev-VM proof: `the GLES renderer is up`
  over llvmpipe); new `packages.scoot-gpu` carries `gpu-scanout` (`ldd`
  shows libgbm); a `dlopen` pre-flight turns a missing libEGL into the
  designed startup error (proven live with the library hidden). Mesa ICDs
  stay the host OS's by decision. Test 4 on Asahi stays open; roadmap row 6
  fixed toward done-on-paper at the same time (also closes #178.6).
- [`packages.scoot` ships `scootctl` too](./resolved/scoot-package-ships-scootctl-done.md)
  — RESOLVED 2026-09-21: `cargoBuildFlags = [ "-p" "scoot" ]` (packaging
  matches the docs; the split is the documented design and the `scootctl`
  derivation already shows the shape). `packages.scoot` bin carries only
  `scoot` on both systems (the 529440-byte redundant `scootctl` is gone,
  byte-count matched to #172's pre-fix `ls`); wrappers, `apps` and module
  checks all still resolve, live IPC proven against the packaged pair.
- [home-manager `sessionScript` vs `configFile` override](./resolved/hm-session-script-path-done.md)
  — RESOLVED 2026-09-21 (gh #174): script path derived from `configFile`'s directory (`scoot/session.sh` by default, byte-identical), `hmRelocated` pins the pairing.
- [CI never exercises the Nix packaging](./resolved/ci-nix-packaging-done.md)
  — RESOLVED 2026-09-21 (gh #173): `nix flake check -L` every PR (Linux + macOS jobs, each its own systems) plus `nix fmt --check` over all tracked `.nix`; `nix build .#scoot .#scootctl` main-only (5m10s cold, zero cache reuse — per-push tax declined).
- [NixOS session entry can't launch a shell](./resolved/nixos-session-command-done.md)
  — RESOLVED 2026-09-21 (gh #171): `session.command` (nullOr str, default null = bare `--tty`, byte-identical) takes the full `Exec=` line — append shape and wrapper-script path both expressible, HM `sessionScript` pairing documented both sides.
- [Flake polish: `homeModules` alias, Darwin default, nixfmt](./resolved/flake-polish-done.md)
  — RESOLVED 2026-09-21 (gh #175): `homeModules` alias over legacy `homeManagerModules` (both evaluate identically), Darwin HM default `null` (files-only; explicit `package` still wins), `compositor-deps.nix` formatted, formatter un-aliased to `pkgs.nixfmt`. Flake-only, zero `.rs`.
- [Nix `src = self` invalidates the build on doc-only edits](./resolved/nix-src-fileset-done.md)
  — RESOLVED 2026-09-18: `src` is a `lib.fileset` union of `Cargo.toml`,
  `Cargo.lock`, `crates/` (every other `./` read in the flake re-verified
  eval-time); the old working-tree copy also dragged `target/` + `.git`
  along, so `src` drops from ~1.1 GB to ~3.3 MB in-store and doc/target
  edits no longer move the drv. Proven by `nix build` (aarch64-darwin).
- [`x86_64-darwin` in `systems` breaks `flake check --all-systems`](./resolved/flake-x86-darwin-system-done.md)
  — RESOLVED 2026-09-18 as deliberate exclusion: the pinned nixpkgs
  (26.11) throws at `legacyPackages.x86_64-darwin` before any per-system
  definition is reached, so the system is dropped (a whole-tree repin to
  26.05 declined on cost); the Darwin client path is arch-independent,
  so `cargo build` from source stays open on Intel Macs.
- [`flake.nix` description drift](./resolved/flake-description-drift-done.md) (nit)
  — RESOLVED 2026-09-18: top-level `description` names both (compositor on
  Linux, `scoot msg` client on macOS); `meta.description` is per system
  (Linux reads `crates/scoot/Cargo.toml`, Darwin names the client). A fully
  single-sourced fix is loader-impossible (top level must be a syntactic
  attrset, `description` a string literal — both proven live), so the top
  level stays one literal by fiat.
- [`smoke-test.sh` hardcodes temp paths](./resolved/smoke-test-temp-prefix-done.md) — RESOLVED 2026-09-17: every temp path derives from `$SMOKE_PREFIX` (unset = byte-identical legacy defaults); two concurrent runs with different prefixes proven green
- [`smoke-test.sh` background check samples the cursor under `--tty`](./resolved/tty-background-not-painted-done.md)
  — RESOLVED 2026-09-16, together with the `--tty` entry above that had
  independently (and wrongly) filed the same failure as a compositor bug.
- [Enhanced hardware/DRM testing ideas](./testing/hardware-testing-ideas.md) (research)
- [Dispatch flood tests die on fd-pressure kills under a pressured process table](./resolved/dispatch-flood-fd-pressure-flake-done.md)
  — RESOLVED 2026-09-20 (test-only): both dispatch halves run under the icon test's treatment (raised `RLIMIT_NOFILE` ceiling with verified headroom — 896 free for the 512-retaining fill, 384 for the fd-less single-pixel flood — plus budget-cause pinning by message); the single-pixel flood now takes the shared `FD_FLOOD_LOCK`. The `immed`   twin reproduces identically and is left for a follow-up per the ticket's scope.
- [Two more dispatch floods in the same fd-pressure class (`immed`, second-client)](./resolved/dispatch-flood-remainder-flake-done.md)
  — RESOLVED 2026-09-20 (test-only): the `immed` twin takes the same two-line treatment (headroom + budget-cause pin by message), second-client takes headroom only (it asserts the fill succeeds); both locks and checks on the test thread.
- [Pin the budget cause by message in the two bypass-loop cap tests](./resolved/bypass-loop-cap-cause-pin-done.md)
  — RESOLVED 2026-09-20 (test-only): per-call-site message pins (`maximum of 512 live buffers`) with the helper's code-only stance left standing for its other users; proven vacuous-then-biting (`prlimit 650` green-while-guarding-pressure pre-fix, loud red post-fix).
- [`msg_broken_pipe`'s unbounded `accept()` wedges the suite on a transient connect failure](./resolved/msg-broken-pipe-accept-hang-done.md)
  — RESOLVED 2026-09-20 (test-only + one config stanza): 30s accept deadline in the fixture (loud `TimedOut` naming the cause, `resume_unwind` across the join) plus `.config/nextest.toml` backstop (`period = 60s, terminate-after = 2`, ~15x above the 7.72s measured max); `Command` timeout deliberately left to the backstop.
- [Bind-before-spawn in the `msg_broken_pipe` fixture](./resolved/broken-pipe-bind-race-done.md)
  — RESOLVED 2026-09-20 (test-only): `serve_once` takes an already-bound listener, so program-order `bind` → spawn closes the bind-vs-connect race (before-control 21/22 with one 30.01s race-red, after 22/22 green); 30s deadline and 2s regression test unchanged.
- [Test socket paths overflow macOS `SUN_LEN` under a long `$TMPDIR`](./resolved/fixture-socket-sun-len-done.md)
  — RESOLVED 2026-09-20 (test-only): `socket_path` now builds `scoot-ep-{pid}-{nanos-lo32-hex}-{tag}.sock` (32–34-char filenames, 81–83 total against the dev Mac's 49-byte `$TMPDIR`); pre-fix `SUN_LEN` failure reproduced on the Mac, 4/4 green after, full Linux gate green. macOS CI is check-only, so coverage is local-only by construction.
- [No CI: every verification run is manual and self-reported](./resolved/ci-test-run-done.md)
  — LANDED 2026-09-19 (PR #140): `.github/workflows/ci.yml` runs fmt,
  clippy, `cargo nextest run --workspace`, `cargo test --workspace`, the
  `--headless` smoke test, and an `ldd` pair asserting the default build
  links no `libgbm`/`libEGL` while the `--features gpu-scanout` build does
  — all through `nix develop`; plus a macOS `cargo check` of the `scoot
  msg` client. 4m30s warm, 8m05s cold. What CI cannot cover (`--tty`, a
  DRM/VT seat, GPU hardware, `--nested`, performance) is named in the
  workflow's own header.
- [Extract a shared test harness; split the largest test files; adopt `cargo-nextest`](./resolved/large-test-file-organization-done.md)
  — RESOLVED 2026-09-16: `compositor/test_support.rs` now carries the
  real-client harness the five largest suites each reimplemented; the two
  biggest are split by concern; `cargo-nextest` is on the dev VM and in the
  documented verification set. Same tests, same assertions, 707 fewer lines
  of duplication.

### Found reviewing the webtop fixes (2026-09-19)

- [Every host configure is acted on immediately](./resolved/coalesce-host-configures-done.md)
  — RESOLVED 2026-09-20: a later configure only overwrites an
  allocation-free queue slot and the next render drains at most one size per
  frame tick (live flood: 81 modes pre-fix, 25 post-fix release); a failed
  resize's never-rendered size is `delete_mode`d off the failure path. The
  unbounded product and the in-place GLES resize stay open as stated in the
  record.
- [`smoke-test.sh` defaults to a shared target dir](./resolved/smoke-test-binary-default-done.md)
  — RESOLVED 2026-09-20: SCOOT/SCOOTCTL default to the invoking tree
  (`$CARGO_TARGET_DIR`, else the script's own repo) with SCOOTCTL always
  paired next to SCOOT (same build); any unresolvable binary fails loudly
  before launch, and the header prints path + source + mtime. Silent
  wrong-binary runs are now either loud failures or auditable in the log.

### Field reports from the webtop deployment (2026-09-19)

Both filed as GitHub issues from running scoot nested inside webtop — the
deployment target `README.md` names. **Both RESOLVED 2026-09-19**, shipped
together as one PR since they are one field report.

- [`--nested` ignores every configure after the first](./resolved/nested-follow-host-resize-done.md)
  (issue #144) — RESOLVED. A `--nested` session now follows the host
  window's size for its whole life, not just the first configure. The
  design question the issue asked is answered in the code rather than
  inferred: two entry points, `Host::apply_first_configure` (a failure is
  fatal — nothing is on screen yet) and `Host::apply_resize` (a failure
  logs and keeps the session at the size it was), over one private core.
- [A clean disconnect logs at INFO, flooding an idle session](./resolved/clean-disconnect-log-flood-done.md)
  (issue #145) — RESOLVED. Only the `ConnectionClosed` arm moved to
  `debug!`; all the diagnostic value the logging was added for is in the
  `ProtocolError` arm, which stays at `warn!`. Ten connect/bind/disconnect
  cycles now add zero lines to an `info`-level log.

### Requested 2026-09-19

Filed together and **sequenced behind the `scootctl` split and milestone 6**
at the user's direction. The `blocked:` field on each records that ordering;
none of them is technically blocked, so the sequencing is a choice and can
be revisited.

- [Multi-output: more than one monitor at a time](./core/multi-output.md)
  — **HIGH**, and almost certainly milestone-sized rather than
  backlog-sized. `README.md`'s "Not yet" list leads with it. Its
  [foundation](./resolved/multi-output-foundation-done.md) landed
  2026-09-19: `State.output` is now an id-keyed collection, and `--headless
  --outputs N` gives two virtual screens to test against, so what remains is
  making the protocols correct across them. The scope is enumerated by
  `outputs.rs`'s `Outputs::primary` doc (layer-shell wants four *different*
  per-output behaviours, not one) and by `session-lock-per-output-done.md`,
  which was resolved as single-output *pins* rather than as multi-output —
  including that `locked` must wait for every output's blanked frame, which
  is the security-relevant one. Promoted 2026-09-20 to **Milestone 19**
  ([plan](../roadmap/19-multi-output.md), in progress, phases A–D
  VM-testable, E hardware-gated) — this entry stays the detailed spec.
  Phases A–F have landed (render, layer shell, lock, workspaces, moves),
  E included (2026-09-25, `--tty` drives every connected monitor).
- [Per-output scale/mode configuration surface](./core/per-output-scale-mode.md)
  — deliberately left out of milestone 19: design answered 2026-09-22
  (`[[outputs]]` config shape, per-output scale enumeration, apply/test
  follows config) and build-ready as a spec — still needs two-connector
  hardware to verify against. Do not build blind.
- [Workspace shortcuts: no numbered bind, and no move-to-index action at all](./resolved/workspace-index-keybindings-done.md)
  — RESOLVED 2026-09-20: `Super+1`..`9` focuses and `Super+Shift+1`..`9`
  carries-and-follows (new `MoveWindowToWorkspaceIndex` core/IPC/config
  action mirroring `FocusWorkspaceIndex`'s ignore-out-of-range rule); agent
  window placement is absolute, not stepped.
- [An action that sets a column's width directly](./resolved/set-column-width-done.md)
  — RESOLVED 2026-09-21 (gh #204, PR #206): `set-column-width N` (0-based
  into `[layout] column_widths`, out-of-range ignored) through core → IPC →
  shared grammar → docs; no default bind, no protocol bump, toggle still out.
- [No config reload](./resolved/config-reload-done.md) — RESOLVED 2026-09-20:
  `scootctl reload` re-applies gap, appearance and keybindings live with an
  applied-vs-refused reply; scale/gpu/renderer/autostart/cursor stay
  startup-only and refuse with a message. Failed reloads keep the running
  config; `--tty` VT recovery binds are un-strippable.
- [SIGHUP trigger for config reload](./resolved/reload-sighup-trigger-done.md)
  — RESOLVED 2026-09-21: `kill -HUP` drives the same shared reload path with
  no reply channel (the applied/refused summary goes to the log); default
  terminate disposition fully replaced, composes with the SIGCHLD reaper,
  children keep default HUP. inotify stays out; `scootctl` gets no handler.
- [Consuming scoot from another flake, and the missing home-manager module](./resolved/flake-consumer-and-home-manager-done.md)
  — RESOLVED 2026-09-20: README Install carries the consumer snippet and
  `docs/nix.md` owns the fuller section; thin `programs.scoot` modules on
  both sides (free-form `settings` via `pkgs.formats.toml`, opt-in
  additive login-screen entry defaulted off, portals.conf installed by
  default). Hermetic eval + content checks under `nix flake check`;
  module-rendered configs proven live through a real `--headless`
  session (empty, binds-with-quoting, wrong-type fail-safe).
- [No way to emit a default config file](./resolved/default-config-command-done.md)
  — RESOLVED 2026-09-20: `scoot --print-default-config` emits a commented
  starting file to stdout, generated from the live defaults, parsing back to
  all of them exactly.
- [`--print-default-config --write`: refuse-to-overwrite convenience](./resolved/default-config-write-done.md)
  — RESOLVED 2026-09-21: the follow-up the entry above left open. `--write`
  places the same emission at the default location (parents created, `0o600`,
  symlinks refused as themselves) and refuses loudly — one winner under
  concurrency — instead of overwriting; no custom path, no `scootctl`
  surface.
- [`scoot --help` hides `--renderer` under `--tty`](./resolved/cli-help-tty-missing-renderer-done.md)
  — RESOLVED 2026-09-20: one-line `--help` fix (`--tty` usage line names
  `[--renderer pixman|gles]`, no behavior change) plus a usage-vs-parser
  pinning test; the stale unconditional `--tty`-warns-and-keeps-pixman
  claim corrected everywhere it appeared.
- [CPU vs GPU rendering, measured on a real GPU](./resolved/gpu-vs-cpu-measured-done.md)
  — RESOLVED 2026-09-21 on the user's Apple M2 under Asahi Linux. The
  extrapolation held and then some: GPU scanout costs **4.2–5.1x less
  compositor CPU** under damage (llvmpipe had it ~1.5x *worse*), draws the
  same pixels bar the cursor's antialiasing, and draws ~0.2 W *less* power —
  for +7–16 MB RSS. Offscreen GLES reached **parity** with pixman where
  llvmpipe measured 18–31x. Both tiers use zero CPU at idle. Two
  methodology lessons: the VM's unpaced-injection benchmark measures nothing
  on hardware 14x faster per IPC round trip (damage gets coalesced away), and
  "confirm which tier is live" nearly failed because the compositor
  colourised redirected logs.
- [Rounded window corners](./resolved/rounded-window-corners-done.md) — the cost
  is not the corners, it is that a rounded window is no longer opaque, so
  what is behind it can no longer be skipped. Measure with *overlapping*
  windows; a single-window benchmark will show nothing. Milestone 6 changes
  the calculus, which is why it is sequenced after it.
- [Fractional-scale ring-hole drift in the painted-ring origin refresh](./resolved/ring-hole-fractional-drift-done.md) — RESOLVED 2026-09-21: the reuse check compares `plan.inner`/`plan.outer` too (one more comparison on the already-computed plan, still allocation-free); pinned by the ticket's x=1→2 instance fail-first plus an in-harness brute-force sweep (1.25/1.5/1.75/1.33) and an integer-scale never-repaints test.
- [Ring and content corners do not line up at fractional scale](./resolved/rounded-ring-content-fractional-mismatch-done.md)
  — RESOLVED 2026-09-21 (gh #205, PR #207): ticket hypothesis falsified
  (2.0 fails like 1.5); real cause was `src: None` defaulting to logical
  size on physical-pixel strip buffers — explicit full-buffer `src` both
  strips, GLES fixed by the same change.
- [dma-buf capture buffers for `ext-image-copy-capture-v1`](./protocols/screencopy-dmabuf-capture.md)
  — filed out of milestone 6 stage 4, which was expected to cover it and
  should not have: importing a client's dma-buf and *rendering into* one are
  different capabilities. Wants a write path that does not exist, the
  renderer's `dmabuf_render_formats` rather than its import set, a `DrmNode`
  the GPU-less target does not have, and a fix for the bound-target cache
  eviction `dmabuf.rs` warns about. No measured client need yet.
- [README rewrite: helpful, concise, aimed at the user](./resolved/readme-rewrite-done.md)
  — RESOLVED 2026-09-20: the third item of this queue, worked as an audit +
  tightening pass rather than a rewrite (PR #132 had already given
  `README.md` its front-door shape). Every checklist item verified by
  running, not reading: keys table vs defaults, config example vs
  `Config::default()`, both `--help` outputs live, all 31 protocol
  versions against a live `wayland-info` dump, all 15 README anchors
  mechanically. Five doc corrections, one precision restructure (dmabuf's
  own subsection anchor), and one code bug filed-not-fixed (`scoot --help`
  hides `--renderer` under `--tty`).
- [README + docs consistency re-audit](./resolved/readme-rereview-done.md)
  — RESOLVED 2026-09-21 (coordinator-directed, no gh issue): every
  user-facing surface since PR #159 re-verified against `README.md` and
  `docs/` (both `--help` outputs and `--print-default-config` run live,
  defaults vs `Config::default()`, binds vs defaults, IPC actions vs
  `scoot-ipc`, protocol versions unchanged). Twelve of thirteen
  surfaces already consistent; six fixes — README "Not yet" retitled
  past one-output, three stale one-output sentences in `docs/`,
  the `nix.md` live-defaults paste (missing `corner_radius`,
  `cursor_theme`/`gpu` shown as defaults while unset), and one word
  in the CI paragraph.

### Found asking how a session starts its own programs (2026-09-19)

One question — *"how do we handle startup exec of other processes like
Waybar, fuzzel, browsers?"* — turned up one confirmed bug, since fixed
([every spawned child became a zombie](./resolved/spawned-children-never-reaped-done.md),
resolved 2026-09-20: a `SIGCHLD` handler plus a tracked-pid drain reaps
exactly what `State::spawn` started), and two gaps. They share a mechanism
(what a session owes the programs inside it) but are separately actionable.

- [`XDG_CURRENT_DESKTOP` is set nowhere, so a portal has no backend to pick](./resolved/session-environment-and-portals-done.md)
  — RESOLVED 2026-09-20: `XDG_CURRENT_DESKTOP=scoot` exported unconditionally
  plus `XDG_SESSION_TYPE`/`XDG_SESSION_DESKTOP` filled where no logind set
  them (one pure `resolve`, applied in-process and per spawned child, pinned
  by unit + live-child tests and a smoke-test section); activation
  propagation decided as the session script's job (documented with both
  shapes); `resources/scoot-portals.conf` shipped (`default=gtk`,
  capture via `wlr` — xdg-desktop-portal-wlr ≥ 0.8.0 speaks
  `ext-image-copy-capture`, and its Screenshot portal shells out to ext-only
  `grim`, so the ticket's "no backend speaks ext" premise is superseded with
  sources cited). Flake install wiring filed as the packaging remainder;
  live portal proof impossible on the dev VM (no portal stack installed).
- [Nothing documents how to start a bar or a launcher, and `--` takes one command](./resolved/startup-programs-and-autostart-done.md)
  — RESOLVED 2026-09-20: "Starting a session" documented (`session.sh` +
  webtop `/defaults/startwm.sh` variant, in `docs/configuration.md` with a
  README pointer), `[autostart] commands` built as a flat list of action
  strings through the shared `scootctl::action` parser (fail-open per
  entry, entries first in file order then the `--` command, no supervision),
  home-manager left as a pointer for the flake ticket. The entry's design
  argument stands: config is *state*, the session script is *behavior*, and
  the reason that line holds is that one vocabulary parses in three doors.
- [Docs gaps found converting a real NixOS config](./resolved/nixos-conversion-docs-gaps-done.md) — RESOLVED 2026-09-21 (gh #178): CHANGELOG session-identity strings + repo-move notice, a migration section in `docs/nix.md`, a packaged-renderer pointer on the live-defaults reference, an end-to-end session example on the new `session.command` surface, the `scoot msg reload` one-liner. Two premises stale at fix time, corrected not preserved: the roadmap row (fixed by #198 already) and the renderer premise (fixed by #198's GPU tiers).

### GPU tier completion (filed 2026-09-22)

Survey of what the README/`docs/tty.md` still name as missing on the
optional GPU tier. pixman stays the default and GPU-free operation stays a
hard requirement; this is "the GPU tier is complete and safe", not "GPU
becomes the primary path". Work order: exporter → client fullscreen + format gate → candidates →
capture-cursor → resize-in-place → syncobj → nested dmabuf → VRR.
`screencopy-dmabuf-capture` (above) stays gated on measured need.
- [Widen the scanout framebuffer exporter](./resolved/gpu-direct-scanout-exporter-done.md) — RESOLVED 2026-09-22: `NodeFilter::All` (a `Node` filter is inert: client dma-bufs carry no node without v6 `set_sampling_device`); force/refusal halves pinned and watched firing live on the dev VM under an uncommitted `ANY`-bit experiment. Found the next gate instead of direct scanout: the swapchain format match.
- [Primary-direct format gate](./resolved/gpu-primary-direct-format-gate-done.md) — RESOLVED 2026-09-23 (PR #228, with the candidates ticket's primary-plane half): a fullscreen window covering its output goes primary-direct, flags decided per frame (`ALLOW_SCANOUT | ANY` only for an eligible frame -- unlocked, covered, no capture stream, nothing translucent or rounded -- no primary bit on any other frame); a capture-streamed output stays composited (forcing per frame measured a net loss). Live on the dev VM at default config; ~50x less compositor CPU than llvmpipe compositing there.
- [Honour client fullscreen](./resolved/client-fullscreen-done.md) — RESOLVED 2026-09-22: a per-window fullscreen state in `scoot-core` (covers the output while its column is focused, keeps its slot, restores exactly), wired to xdg `set_fullscreen`, the wlr request + state bit, IPC `toggle-fullscreen`/`set-fullscreen` and `Super+f`; the `top` layer is hidden from drawing, pointer and keyboard under it. Also fixed frame learning pairing a commit with the wrong configure.
- [Scanout candidates + scanout-tranche feedback](./resolved/gpu-scanout-candidates-done.md) — RESOLVED 2026-09-23 (PR #230; the primary-plane half with the format gate above, PR #228): the fullscreen window covering an output gets per-surface feedback with a `scanout` tranche (the advertised pairs the primary plane accepts, `target_device` the display device) while the output may go direct, reverted at once when the covering window changes and, for a lock/stream/translucency, on the first frame after a 2 s hold; `wp_presentation` reports `zero_copy` for the surface scanned out directly. Overlay-plane candidates split out below.
- [Captures lose the pointer on the scanout tier](./resolved/capture-cursor-parity-done.md) — RESOLVED 2026-09-23: the pointer in a capture is the request's on every backend and renderer -- `paint_cursors` honoured, IPC `screenshot` `cursor` field (default drawn in, `--no-cursor`) -- by re-rendering the cursor's region from the frame's own elements where the frame does not already match; the scanout tier reads what rode a plane from the `DrmCompositor`. Headless/nested screenshots now show the pointer (they never drew one; the dispatch assumed they did). An overlay-planed cursor's possible underlay hole is filled whatever was asked; a stream's region buffers are pooled; a no-cursor session is not re-served on pointer-only moves. Dev VM: arrow pixels identical across the scanout and dumb tiers, exactly one cursor through primary-direct captures; a 0.4-0.9 ms region per pointer-requesting capture on the llvmpipe scanout tier.
- [Captures allocate a whole frame per capture](./core/capture-whole-frame-allocations.md) — low (computer use): the read-back and the IPC copy allocate multi-MB buffers per capture, and screenshot latency tracks their page faults more than any capture work (found reviewing PR #231).
- [GLES resize in place](./resolved/gles-resize-in-place-done.md) — RESOLVED 2026-09-23 (PR #232): a GLES resize reallocates only the renderbuffer, keeping the EGL context, shaders, client textures and device; a size over the context's limit is refused up front; any other failure falls back to a pinned rebuild, then refuses as before. Dev VM (LTO off): 3.95 ms → 15.7 µs per resize (pixman 11.3 µs); `--nested` drag 17.0 → 14.7 ms CPU per size, the rest being llvmpipe's frame at the new size.
- [Explicit sync, `linux-drm-syncobj-v1`](./resolved/linux-drm-syncobj-done.md) — RESOLVED 2026-09-23 (PR #233): offered on the GPU scanout tier only, where the display device (or failing that `/dev/dri/renderD128`) passes Smithay's syncobj-eventfd probe (every `renderD*` in order); acquire points waited on with eventfd blockers (signalled points: one ioctl, no eventfd; sources removed on surface destroy and disconnect; 64 outstanding waits per client, then `wl_display.no_memory`); release points held for composited frames until their flip (frames never shown release at once, no fence wait on pause/activate), direct buffers already held until off the plane; 128 live timeline objects per client (since 2026-09-24: 128 retained timeline fds, see client-held fd bound below). Dev VM: global on the GPU tier only, held commits and releases seen live across a VT switch, a stalled client isolated, both bounds killing only the offender, fds back to baseline, CPU unchanged. Real GPU and real Vulkan/NVIDIA clients: `Asahi.md` Test 7.
- [Smithay leaks a syncobj handle per timeline import](./resolved/syncobj-handle-leak-done.md) — RESOLVED 2026-09-24 (PR #233) in scoot by repinning Smithay to the scoot-sh fork (`43f50eb2` = upstream `0ff0098` + one `Drop`): the import-and-destroy loop went from +4332 kB to +0 kB, and abandoned waits after client exit from +3668 kB to +84 kB. Upstream is still open; nothing is filed from this project (`docs/forks.md`).
- [Repin Smithay from the fork back to upstream](./core/smithay-fork-repin.md) — low, blocked on an upstream rev carrying the fix.
- [Client-held fds no cap counts](./resolved/client-held-fd-bound-done.md) — RESOLVED 2026-09-24 (PR #236): dma-buf planes added to params objects not yet created are counted per client (32, 8 under fd pressure; `wl_display.no_memory` past it), released on consume, destroy or disconnect; the syncobj timeline cap (128, 32 under pressure) now counts the timeline fds scoot really holds, observed through the fd itself (the ledger then in `drm_syncobj/retained.rs`, since generalised into `client_fds.rs` by PR #239), so points on destroyed timelines count until they go. Dev VM: both 927-fd shapes now refused at the cap (params at the 9th object, timelines at surface 65), newcomers and `scootctl` served throughout, real GL/Vulkan clients and an explicit-sync client unaffected.
- [Buffer fds retained past their object](./resolved/buffer-fds-past-their-object-done.md) — RESOLVED 2026-09-24 (PR #239): every fd a client hands scoot that scoot keeps (shm pools, dma-buf planes, syncobj timelines) is counted per client until it really closes, by number with the file's identity (`client_fds.rs`, generalising PR #236's timeline ledger); 512 per client, one 128-fd pressure grace replacing the per-kind ones; the GLES renderer's own copy of each imported plane (llvmpipe keeps one) is measured once per session and charged too. Dev VM: 900 surfaces keeping destroyed buffers (919 fds, newcomers shed on `main`) refused at 512; 200 three-plane surfaces on the `--tty` GLES tier (table full on `main`) refused at 85 buffers, 557 fds at the bound; real clients unchanged.
- [wayland-backend's unbounded received-fd queue](./resolved/wayland-backend-fd-queue-done.md) — RESOLVED 2026-09-25 (PR #241) with the fd-limit raise below: scoot builds against the scoot-sh wayland-rs fork (`docs/forks.md`: 0.3.17 + `a39311b8` + `70f81e00`, via `[patch.crates-io]`), which disconnects a client leaving more than its cap of received fds unclaimed, one eighth of the soft limit clamped 128..=1024: 1024, libwayland-server's default, at scoot's raised limit. Dev VM: the PR #236 probe disconnected at 1036 (18 fds after, everyone served); a stock libwayland client stalled behind a stopped scoot served at 160/600/1000 (the first round's fixed 128 disconnected it). `fd_pressure/tests/backend_queue*.rs` fail if the patch is dropped.
- [Raise the fd limit at startup; children get the original](./resolved/raise-nofile-limit-done.md) — RESOLVED 2026-09-25 (PR #241): soft `RLIMIT_NOFILE` set to min(hard, 65536) (up or down), logged; the fd-table reading is cached (reused for 20x its cost) so an accept storm against a full table costs one readdir (35.6 s freeze → 101 ms); `State::spawn` children get the original back (`pre_exec`); Xwayland raises its own limit (measured), so it is not restored; a 1024 hard limit stays 1024 with a log line and the 128 cap.
- [`--nested` gles presents by dmabuf](./resolved/nested-dmabuf-present-done.md) — RESOLVED 2026-09-24 (PR #235): in a `gpu-scanout` build, a nested GLES session whose host composites on the renderer's own DRM device blits each frame into a host dma-buf (GBM on that device, render node then primary; `zwp_linux_buffer_params_v1.create`; at most three per size, grown on demand; a frame with no free buffer handed over as drawn) instead of reading it back; read-back otherwise, chosen at startup from the host's v4 feedback and logged; a host refusal falls back for good with one WARN. Dev VM (llvmpipe, outer scoot as host): host screenshot byte-identical, CPU-neutral per frame, ~6-8% dearer per resize (software drawing dominates). Real GPU: `Asahi.md` Test 8.
- [GLES captures keep a whole frame each while nothing redraws](./resolved/gles-capture-leaks-a-frame-per-shot-done.md) — RESOLVED 2026-09-24 (PR #238): Smithay queues a dropped read-back buffer (and the bind's framebuffer object) until the next frame's drain, and a capture of a static screen reached no drain. Both GLES arms of `Backend::capture` and the GLES capture-cursor region render now drain once the capture returns. Dev VM: 120 `scootctl screenshot --no-cursor` or `grim` captures went from +750–763 MB to flat after one frame, on `--nested`, `--headless`, the `--tty` GPU tier and the `--nested` dma-buf tier. p50 8.8 → 7.9 ms.
- [VRR on the scanout tier](./core/gpu-vrr.md) — low, blocked on a VRR-capable display (the Asahi M2 eDP panel is confirmed not to be: no `vrr_capable` property, 2026-09-25).

### Requested 2026-09-24
- [Floating windows (dialogs auto-float, window rules, toggle, move/resize)](./resolved/floating-windows-done.md) — RESOLVED 2026-09-25 in two PRs. PR 1 (#242): the floating layer, auto-float by `xdg-dialog-v1`/parent/fixed size, `[[window_rule]]`, the toggle and focus switch, IPC. PR 2 (#243): `[floating] modifier` (Super) + left/right drag moves/resizes, the client's own `xdg_toplevel.move`/`resize` honoured on its held press (tiled windows' ignored), IPC `move-floating`/`resize-floating`, cross-output moves, a window's dialogs always drawn above it (the re-review's fullscreen-game case), resize configures paced to client acks; no allocation per pointer motion (a resize allocates only for its configures, ~11 each, about one per client ack).

### Found measuring scootbg's dependencies (2026-09-26)
- [A tiny wl_shm buffer upscaled by wp_viewporter fades at its edges under pixman](./core/shm-viewport-upscale-edge-fade.md) — low: a 1×1 XRGB shm buffer stretched to the output samples transparent past its edge (corner alpha 0.25). Single-pixel buffers are unaffected; other clients upscaling a small shm buffer are.

### From the PR #264 review (2026-09-26)
- [Withhold frame callbacks from layer surfaces nobody can see](./core/frame-callbacks-for-hidden-surfaces.md) — low: every mapped layer surface gets `frame` on every render with no occlusion check, so an animated wallpaper under a fullscreen window never learns it is covered. Blocks scootbg's animated wallpapers (its milestone 2), not v1.

### From the PR #243 review (2026-09-25)
- [No per-client toplevel cap: floods or chains of toplevels stall arrange](./core/per-client-toplevel-cap.md) — medium: `arrange` runs per frame per output and is multi-ms at a few thousand windows; floating a transient window runs a full `arrange` (`parent_centre`), so n transients are O(n^2) (4000-chain build 7.3 s debug), and `recentre_floating` does the same per floating window on an output change. Model a cap on the popup/subsurface caps (64).

### From the PR #239 review (2026-09-24)
- [Many light connections can still pressure the table and shed scootctl](./core/pressure-many-light-connections.md) — medium; cheapest fix is a lower pressure line for IPC accepts. Since PR #241's raised table it takes 64 idle connections parking 1024 fds each (no objects) to fill it, measured; still 7 where the hard limit is 1024.

### From the gh #205 re-verification (2026-09-24)
- [Clip and ring the committed size; send tiled states](./resolved/clip-to-committed-size-done.md) — RESOLVED 2026-09-24: layout windows are told they are tiled on all four edges (default foot now fills its slot: 958x1068 → 966x1083 at 1.5), and the rounded clip, both rings and IPC `rect` follow the committed size clamped to the slot (`drawn.rs`), so a client that still draws short (GTK 4 dialogs, mpv) is clipped and ringed where it drew. Default foot at 1.0/1.5, headless and `--tty`: all four corners pass (right and bottom failed on `main`). The review found that a re-map had lost size, tiled states and the decoration mode; they are now rebuilt from the layout. Residual filed below.
- [A client that rounds its own corners leaves a sliver between its corner and the ring](./core/client-rounded-corners-vs-ring.md) — medium: libadwaita dialogs round wider than scoot (53–59 background px per corner at 1.5). They float now (floating windows PR 1), which does not change the radius mismatch.
- [Ring outer corners have square shoulders](./resolved/ring-outer-corner-shoulders-done.md) — RESOLVED 2026-09-24: the painted ring's side bars span only the rows between its strips (from the strips' own physical geometry), so the outer edge is a concentric arc in every row. Pixman and GLES, 1.0–2.0.

### GPU tier, after primary-direct (2026-09-23)
- [A/B resource usage vs niri](./resolved/niri-ab-benchmark-done.md) — RESOLVED 2026-09-24 (PR #237, dev VM half), at the user's direct request: both compositors nested in the same cage host on llvmpipe, three rotating rounds, results and caveats in [`docs/benchmarks.md`](../benchmarks.md). The real-GPU half is the next entry.
- [A/B vs niri on a real GPU, `--tty` included](./testing/niri-ab-real-gpu.md) — low: **run 2026-09-25** on the Apple M2 (`Asahi.md` Test 9, `docs/benchmarks.md`); only input-to-present latency remains, which needs a method first.
- [GLES tier advertises only LINEAR dma-bufs](./resolved/gles-dmabuf-full-formats-done.md) — RESOLVED 2026-09-23 (PR #229): under `gles` the feedback is the driver's own import set (every fourcc at every explicit modifier, external-only YUV included), `Invalid` never offered next to explicit layouts (an implicit YUV buffer draws the wrong colour, measured), pixman byte-identical. On llvmpipe: 57 formats at `LINEAR`, `NV12`/`P010`/`YU12`/`YUYV` imported through `create_immed` and drawn correctly from dumb buffers. Real GPU: `Asahi.md` Test 6.
- [Scanout-tranche feedback + `zero_copy` flag](./resolved/gpu-scanout-candidates-done.md) — RESOLVED 2026-09-23: see the GPU tier completion entry above. Dev VM: tranche `XR24`/`AR24` at `LINEAR`, sent/reverted live, `zero_copy` on exactly the frames Smithay scanned out directly. Real GPU (does a GL client reallocate into it and go direct): `Asahi.md` Test 6 Part C.
- [Windows on overlay planes](./core/gpu-overlay-window-candidates.md) — low, **unblocked 2026-09-25**: Asahi's `apple,dcp` has one overlay (zpos 1, `LINEAR` only, alpha RGB + YUV formats, no `XR24`) and no cursor plane.
- [A composited cursor blocks primary-direct](./core/gpu-direct-blocked-by-composited-cursor.md) — medium, found on Asahi 2026-09-25: with no cursor plane, a visible pointer denies every fullscreen window a primary attempt; mpv went direct once it hid its pointer (~60% less compositor CPU). A cursor on the overlay needs a Smithay-fork change.
- [mpv `--vo=gpu` intermittently black/striped under GLES](./resolved/gles-mpv-shm-black-done.md) — CLOSED WITHOUT CODE 2026-09-26: rate loops (N=20, both renderers) plus a pool-bytes-vs-screenshot proof show the dots and the black are states of mpv/Mesa's own shm present buffers on llvmpipe (pre-present, mid-render tile wavefront), displayed faithfully by both renderers; well-behaved GL shm clients (es2gears) and `mpv --vo=wlshm` are solid. Harness in `scripts/mpv-shm-rate/`.

### Found reviewing client fullscreen (2026-09-23)
- [Flake: xwayland DISPLAY test read "" instead of unset](./testing/xwayland-display-env-flake.md) — low: seen once under GLES nextest.
- [Deep popup chains crash or freeze the compositor](./resolved/popup-depth-bound-done.md) — RESOLVED 2026-09-23 (PR #226): popup chains are capped at 64, and the ways an open chain could be re-parented deeper are refused (`already_constructed`, `not_the_topmost_popup`, a parent with no live role object); a layer surface may adopt only a fresh parentless popup; a surface made a popup again no longer has its same-flush child inserted under its dead tree node. Every chain and tree node is at most 64 deep.
- [Windows bleed onto the neighbouring output](./resolved/windows-bleed-across-outputs-done.md) — RESOLVED 2026-09-23 (PR #224): a window is drawn and hit only on the output it is placed on (render gather, ring and both hit sites); popups go with their parent. Also fixed: the ring and rounded clip never drew right on outputs after the first (global coordinates).
- [Popup constraint adjustment](./resolved/popup-constraint-adjustment-done.md) — RESOLVED 2026-09-23 (PR #225): popups are flipped/slid/resized into their output (a window's: usable area, whole output while it is fullscreen; a layer surface's: whole output), relative to the immediate parent, at the initial configure and on `reposition`. Also fixed: a popup whose parent chain loops back to itself froze the compositor; it is now refused.
- [Reactive popup re-constraining](./core/popup-reactive-reconstrain.md) — low: a `reactive` popup is not re-fitted when its window scrolls or an output changes while it is open.
- [Output membership read from geometry](./core/output-membership-by-geometry.md) — low: frame callbacks, `wl_surface.enter` and foreign-toplevel `output_of_window` still use bbox overlap.
- [Frame learning never flushes its own relayout](./core/frame-learning-apply-flush.md) — low: learned minimum waits for the next event.
- [Smoke xwayland step keys on PATH, not the build feature](./resolved/smoke-xwayland-step-feature-gate-done.md) — RESOLVED 2026-09-25 (PR #244, with XWayland Phases 2+3): the step branches on what the binary logs for `--xwayland` (READY / no-feature warning / spawn failure), runs `--headless` under `MODE=--tty`, and maps and closes an `xeyes` when one is on `PATH`.

### Found implementing the popup depth bound (2026-09-23)
- [Deeply nested subsurfaces overflow the stack](./resolved/subsurface-depth-bound-done.md) — RESOLVED 2026-09-23 (PR #227): no surface is ever more than 64 subsurface levels below its tree's root. Checked in a `dispatch.rs` guard before Smithay links the surfaces or runs its recursive `is_ancestor`, against the new parent's depth *plus the height of the subtree being attached* (a per-surface bound, never lowered), since a subsurface can be re-attached after `wl_subsurface.destroy` or its parent's destruction and a role-less surface can be given children first. Refused as `wl_subcompositor.bad_parent`.
- [Smithay accepts a second `wl_subsurface` for an orphaned subsurface](./core/subsurface-second-wl-subsurface.md) — low: the protocol's `bad_surface` is not raised once the parent `wl_surface` is destroyed; not a depth path.
- [Many desynchronized subsurfaces stall the compositor](./core/subsurface-count-quadratic.md) — medium, measured: `N` sibling desync subsurfaces in one window cost roughly quadratically to create (release: 10000 took 1.15 s, 30000 over 10 s). Pre-existing; breadth, not depth.
- [Many side-by-side popups stall the compositor](./core/popup-count-quadratic.md) — medium, measured (review of #226): popup creation is roughly quadratic in the number open (1954 popups 0.73 s, 5104 5.4 s). Pre-existing.

### Found reviewing nested dma-buf presentation (2026-09-24)
- [`--nested` confirms a lock before the host shows it](./core/nested-lock-confirm-on-present.md) — low: `locked` goes out when the blanked frame is drawn, and a nested frame may be skipped or owed before the host has it (more often since PR #235); a fix needs a bounded wait like `--tty`'s.

### Found by the niri A/B (2026-09-24)
- [Nested scoot presents fewer frames than niri for a ~60 Hz client](./core/nested-frame-rate-vs-client.md) — low: 49.8 frames/s (pixman, 12% of a core) against niri's 54.1 for a `foot` printing every ~16 ms; looks like pacing, not cost. Not investigated.

### Meta
- [Split the CLI out into `scootctl`](./resolved/rename-flex-family-done.md) — CLOSED 2026-09-20: the `flexwm` → `scoot` rename half landed 2026-09-18 (PR #128); the crate split landed 2026-09-20 ([record](./resolved/scootctl-split-done.md)): new `scootctl` lib+bin crate, `scoot msg` kept as a permanent alias, Darwin default is `scootctl`. A status bar stays separate.

### Not-yet removal plan (2026-09-21, macOS excluded)

One entry per README "Not yet" bullet, filed so each has a costed landing
spot. Order across tracks: cursor/`column_widths` + XWayland spike +
TTY-enumeration first (independent); TTY add/remove → placement → default
binds → scale/mode surface; Asahi proof gates scanout planes; autostart
policy → renderer/GPU reword. Batch Asahi trips (multi-CRTC + scanout +
scale/mode) into one hardware session.

- [Restore windows, workspaces and binds when a monitor reconnects](./resolved/output-reconnect-restore-done.md)
  — RESOLVED 2026-09-25 (PR #249): removal files the core's `EvictedOutput`
  keyed by connector identity, a matching add restores workspaces in order,
  binds name the first/second screen. Live Asahi replug unverified (runbook
  in the PR body); no debounce until flap timing is measured.
- [Lock-confirm bound wording, and aging out stale dumb-tier vblanks](./core/lock-vblank-bound-hardening.md)
  — OPEN, low: `await_vblank`'s "late, never early" should say "within one
  bound of the first blank drawn"; `stale_vblanks` entries should age out so
  a driver that never delivers an owed vblank can't freeze a reused CRTC.
  Filed from PR #247's round-2 review.
- [Multi-output remainder: --tty multi-CRTC, placement, default binds](./core/multi-output-remainder.md)
  — OPEN, **HIGH**: milestone 19 phases E–I. G (pointer-output placement)
  + H (default `Super+comma/period` output binds) LANDED 2026-09-21
  (PR #208). E1 (every connector driven at startup) + E2 (per-head
  rendering, per-output lock waits, hotplug add/remove) LANDED 2026-09-25,
  live on the Asahi M2 Air including a physical unplug and replug. Left:
  GPU-tier runtime add and #48 `MoveTo` on hardware,
  [reconnect restore](./resolved/output-reconnect-restore-done.md), and the
  [per-output scale/mode](./core/per-output-scale-mode.md) entry, which
  stays last.
- [XWayland: drops onto X windows do not land](./protocols/xwayland-pointer-focus-x11.md)
  — OPEN, medium (filed by XWayland Phase 4, PR #246): Wayland → X, X → X and
  in-app X drops do nothing because `DnDGrab` targets scoot's `WlSurface`
  pointer focus and XWayland binds no `wl_data_device`; needs an X arm on
  the pointer focus (behaviour-neutral first, benchmarked -- hot path).
- [XWayland support](./protocols/xwayland-support.md)
  — OPEN, low: spike (PR #220), skeleton (PR #221) and mapping + focus gate
  (Phases 2+3, PR #244) landed: X windows tile, dialogs float, rules
  match `WM_CLASS`, fullscreen both ways, override-redirect menus drawn
  unmanaged; an X window takes focus only when nothing is focused, the
  focused window is the same X process, or it redeems its spawn's token
  (`_NET_STARTUP_ID` on the window or its client leader, or X-Resource pid;
  spent whichever rule grants focus), and
  `_NET_ACTIVE_WINDOW` goes through the same gate. Phase 4 (PR #246) landed: the
  clipboard and primary selection cross both ways while an X window is
  focused (never while locked), a paste is refused once the X selection has
  changed hands since it crossed (raises the bar; a forged owner answer is
  still possible), X drags need a press on the dragging client's own
  window (presses on Wayland surfaces protected; on X windows not), and thirteen measured XWM fixes/hooks moved into the Smithay fork;
  X → Wayland drags work, drops onto X windows do not ([pointer focus needs
  an X arm](./protocols/xwayland-pointer-focus-x11.md), medium), no XIM.
  Remaining: capture pins/packaging (5–7). The [WM-failure
  pin](./resolved/xwayland-phase1-wm-failure-pin-done.md) is resolved (its
  rival-claimant recipe cannot work: XWayland admits no X client before the
  WM attaches). Follow-up filed: [bind startup-id redemption to the spawned
  process](./protocols/xwayland-startup-id-race.md) (low: a watching X
  client can race a launched app to its `_NET_STARTUP_ID` within the
  token's 30 s).
- [GPU scanout: cursor + overlay planes](./resolved/gpu-scanout-planes-done.md)
  — RESOLVED 2026-09-22 (coordinator-filed, no gh issue): all three phase-2
  steps landed — cursor plane active where exposed (PR #216), overlay planes
  enumerated per CRTC (PR #217), and `ALLOW_SCANOUT` with its capture fix in
  the same change (direct frames mark the recording, captures served off a
  marked recording force one composite frame first, loud refusal where the
  force cannot draw). No window leaves the primary yet (no candidates, and
  the exporter stayed `NodeFilter::None` until the [exporter
  widening](./resolved/gpu-direct-scanout-exporter-done.md), after which the
  format gate still holds), so the flag is assignment-inert
  and the fix is proven by pins + trace + byte-identity live. Numbers:
  [the measurement entry](./resolved/gpu-vs-cpu-measured-done.md).
- [Config reload: from partial to full](./resolved/config-reload-full-done.md)
  — RESOLVED 2026-09-22 (all phases): autostart spawn-delta (only entries
  the session has not seen run, new spawns once each, reloaded non-spawn
  refused by name, locked reloads defer to the first unlocked one) plus
  renderer/GPU refusals reworded to restart semantics. End state: live
  except `renderer.backend` + `tty.gpu`. SIGHUP inherits; no wire bump.
  Review remainder filed as [autostart
  follow-ups](./resolved/reload-autostart-followups-done.md) — RESOLVED
  2026-09-22 (PR #215): failed spawns refuse by name and retry on the next
  reload (per-entry snapshot via a `spawn`→`act` acceptance bool), the
  locked-skip message promises only a decision, and the
  removal-while-locked cancellation is pinned.
