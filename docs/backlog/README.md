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

### Core / config / rendering
- [`--width`/`--height` are unbounded `i32`s](./resolved/width-height-bounded-done.md)
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
  is the security-relevant one.
- [Workspace shortcuts: no numbered bind, and no move-to-index action at all](./input/workspace-index-keybindings.md)
  — two gaps that look like one. `focus-workspace-index N` exists and simply
  is not bound by default (so `Super+1`..`9` is a keybinding change), but
  `move-window-to-workspace` takes **only** `up|down` — there is no index
  form anywhere, so a window can only be moved one workspace at a time. That
  half is a missing action, and it is an agent-facing gap too.
- [No config reload](./config/config-reload.md) — on `--tty` a settings
  change costs the whole session. Not every field can be re-applied
  (`[output] scale`, `[tty] gpu`, `[renderer] backend` realistically cannot),
  so the honest shape is a partial reload with an explicit list, and a failed
  reload must keep the running config rather than fall back to defaults.
- [Consuming scoot from another flake, and the missing home-manager module](./packaging/flake-consumer-and-home-manager.md)
  — the flake exposes `packages`/`apps`/`devShells` but **no** `overlays`,
  `nixosModules` or `homeManagerModules`. Documenting what already works is
  small; `programs.scoot.enable` is real work with real decisions.
- [No way to emit a default config file](./config/default-config-command.md)
  — `--config PATH` reads one, nothing writes one. Wants generating from
  `Config::default()` rather than a hand-maintained string, or it drifts.
- [CPU vs GPU rendering has never been measured on a real GPU](./rendering/gpu-vs-cpu-measured.md)
  — **HIGH**, blocked on the user's Asahi machine. Every GPU number the
  project has is llvmpipe's. What *is* known: offscreen GLES cost 17–32x
  pixman and scanout costs ~1.5x on the same rasteriser, so the read-back
  was the dominant cost — but "therefore it wins on real hardware" is an
  extrapolation, not a measurement. Runbook is `Asahi.md`'s Test 4. Note
  FPS is the wrong headline for a damage-driven compositor; idle CPU, frame
  cost under damage, RSS and power are the numbers that decide it.
- [Rounded window corners](./rendering/rounded-window-corners.md) — the cost
  is not the corners, it is that a rounded window is no longer opaque, so
  what is behind it can no longer be skipped. Measure with *overlapping*
  windows; a single-window benchmark will show nothing. Milestone 6 changes
  the calculus, which is why it is sequenced after it.
- [dma-buf capture buffers for `ext-image-copy-capture-v1`](./protocols/screencopy-dmabuf-capture.md)
  — filed out of milestone 6 stage 4, which was expected to cover it and
  should not have: importing a client's dma-buf and *rendering into* one are
  different capabilities. Wants a write path that does not exist, the
  renderer's `dmabuf_render_formats` rather than its import set, a `DrmNode`
  the GPU-less target does not have, and a fix for the bound-target cache
  eviction `dmabuf.rs` warns about. No measured client need yet.

### Found asking how a session starts its own programs (2026-09-19)

One question — *"how do we handle startup exec of other processes like
Waybar, fuzzel, browsers?"* — turned up one confirmed bug, since fixed
([every spawned child became a zombie](./resolved/spawned-children-never-reaped-done.md),
resolved 2026-09-20: a `SIGCHLD` handler plus a tracked-pid drain reaps
exactly what `State::spawn` started), and two gaps. They share a mechanism
(what a session owes the programs inside it) but are separately actionable.

- [`XDG_CURRENT_DESKTOP` is set nowhere, so a portal has no backend to pick](./core/session-environment-and-portals.md)
  — the string does not occur in this project's code at all, while `mod.rs`
  already exports four other variables for exactly this kind of reason. Portals are
  how a Wayland browser does screen sharing and file dialogs, and a browser
  is what the webtop target exists to run. Split into a two-line half (export
  it) and a real half (D-Bus activation environment, `scoot-portals.conf`)
  — and the entry is explicit that no portal has actually been watched to
  fail yet, with the check that would settle it. Which backend serves
  ScreenCast is left open rather than guessed: `xdg-desktop-portal-wlr`
  wants `zwlr_screencopy_manager_v1`, which `docs/protocols.md` records as a
  deliberate *non*-implementation, so pointing a config at it would ship a
  backend that fails every request.
- [Nothing documents how to start a bar or a launcher, and `--` takes one command](./config/startup-programs-and-autostart.md)
  — `docs/protocols.md` shows `waybar &` without ever saying where that shell
  runs. Carries the design argument for what "idiomatic scoot config" means:
  config is *state*, the session script is *behavior*, and the reason that
  line holds is that `config.rs:565` parses a `[binds]` value with the same
  `cli::action` the IPC uses — one vocabulary, three doors. An optional
  `[autostart]` would be a list of those same action strings. Supervision is
  argued out of scope (and the webtop target has no systemd, which is the
  wrinkle).

### Meta
- [Split the CLI out into `scootctl`](./meta/rename-flex-family.md) — the `flexwm` → `scoot` rename half landed 2026-09-18 (PR #128); the crate split landed 2026-09-20 ([record](./resolved/scootctl-split-done.md)): new `scootctl` lib+bin crate, `scoot msg` kept as a permanent alias, Darwin default is `scootctl`. A status bar stays separate.
