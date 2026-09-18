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
  — RESOLVED 2026-09-16 (PR #53): `request_activation` and every focus-family IPC
  action now spend the click (`clicked_layer = None`), the way PR #50's own
  `activate` and a window click already did. A launcher or panel that stays
  mapped no longer keeps every keystroke after handing focus away.
- [`ext_workspace.rs`'s cross-client check takes two backend locks per manager, per `wl_output` bind](./protocols/ext-workspace-client-lookup-per-bind.md)
  — pre-existing, same review; `ObjectId::same_client_as` answers the exact
  same question without locking
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
  is). One head for flexwm's one output, read from the same `Output` that
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
  `ext-` protocol. flexwm's own IPC screenshot is unchanged.
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
- [Screen capture: the session count is unbounded](./protocols/screencopy-session-cap.md)
  — found building the output half; why a cap was not simply added
- [Screen capture forces `Xrgb8888`'s undefined fourth byte opaque](./protocols/screencopy-xrgb-alpha-forcing.md)
  — found measuring a review finding on the output half: the forcing is ~13%
  of a release capture and ~77% of a debug one, to set a byte the format says
  is undefined and `grim` demonstrably ignores
- [`xdg-toplevel-icon-v1` pixel-buffer icons are not exposed](./protocols/toplevel-icon-buffers.md) — only the icon name reaches IPC
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
- [Layer surface with no buffer still holds its exclusive zone](./protocols/layer-surface-bufferless-exclusive-zone.md)
- [Unbounded `ext_workspace_manager_v1` binds per client](./resolved/ext-workspace-object-binding-cap-done.md)
  — RESOLVED 2026-09-17 (PR #74): one shared `BindBudget` — 8 binds per
  client across all four globals (ext-workspace, ext/wlr toplevel lists,
  output management), refused with each global's own `finished`; includes
  re-homing the ext list off Smithay state and an idle-deferred refusal
  (backend panics on in-bind destructor events)
- [Lock surfaces are per-output; flexwm has one output](./protocols/lock-surfaces-per-output.md)
- [Unbounded lock surfaces via duplicate `wl_output` binds](./protocols/lock-surface-duplicate-wl-output.md)
- [Lock blanks immediately instead of waiting for the first surface](./protocols/session-lock-blank-timing.md)
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
  [present-skip eats frame damage](./rendering/present-skip-eats-frame-damage.md).
- [A held pointer lock survives a session lock, so a client keeps pointer input while locked](./resolved/pointer-lock-session-lock-done.md)
  — RESOLVED: the lock transition deactivates the held constraint (game sees
  `unlocked`), focus lands on the lock surface, and unlock re-arms through
  the ordinary arrival path. Confinement needed no fix (fail-open +
  leave-deactivation already handled it; pinned).
- [Lock manager global offered to every client](./protocols/session-lock-global-restriction.md)
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
- [The accept loop swallows `EMFILE` and can spin the event loop](./ipc/accept-loop-swallows-emfile.md) — pre-existing, found reviewing the cap
- [The connection cap turns one client's leak into everyone's refusal](./resolved/connection-cap-denies-the-same-user-done.md) — CLOSED 2026-09-17 as an accepted tradeoff (no workload has hit it; per-pid sub-cap stays unbuilt as speculative); kept as the landing spot for a future "the bar cannot connect"
- [A large `msg type` blocks the event loop](./resolved/msg-type-blocks-event-loop-resolved.md) — RESOLVED 2026-09-17: `type` text capped at 16,384 characters per request (sized by measurement; worst case ~75ms), refused naming the limit and the split-workaround, counted in characters not bytes
- [`flexwm msg` panics when its stdout reader goes away](./resolved/msg-client-broken-pipe-done.md) — RESOLVED 2026-09-17 (PR #62): per-write EPIPE handling at every client-binary stdio site, quiet exit 0; compositor unaffected
- [IPC focus actions run a full `apply` even when nothing moves](./resolved/focus-action-no-op-fast-path-done.md) — RESOLVED 2026-09-17 (PR #67): already-there focus actions skip `act` and run only the keyboard half, mirroring PR #54; relative steps deliberately left on the full path

### Input
- [`msg key` hard-codes `_L` modifier keysyms](./resolved/msg-key-modifier-resolution-done.md) — RESOLVED 2026-09-17 (PR #83): modifiers resolve through the keymap probe like `type_text` (either hand's key; toggle-option layouts work); `msg key A` still refuses
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
- [`--width`/`--height` are unbounded `i32`s](./resolved/width-height-bounded-done.md)
  — RESOLVED 2026-09-17: refused past 65535 per axis at parse (the most DRM
  itself can report for a mode axis); `Rect::inset`/`right()`/`bottom()`,
  `scroll_into_view` and the arrange on-screen test saturate.
- [`[binds]` capital letter parses but never fires](./resolved/binds-capital-letter-done.md)
  — RESOLVED 2026-09-17: single ASCII letters fold to lowercase at
  config-parse time with a warning (`"A"` means plain `a`, not `shift+a`);
  `msg key A` still refuses.
- [Per-frame `Vec` alloc in the cursor fallback path](./rendering/cursor-element-per-frame-alloc.md)
- [A `present()` skipped for an in-flight flip consumes that frame's damage](./rendering/present-skip-eats-frame-damage.md) — code-traced only, never observed; scanout (not the read-back image) goes stale until next damage; candidate fix touches the hot present path, so correctly waiting on a live observation. Note: an independent trace during the 2026-09-17 audit disputes this entry's stated mechanism (slot age after a skip is ≥2, so the tracker's history returns the skipped frame's damage and the retry presents) — worth re-deriving before anyone acts on it.
- [A screencopy frame parked for a lock's blank is never re-armed after the vblank confirm](./resolved/screencopy-parked-across-lock-confirm-done.md)
  — RESOLVED 2026-09-17 (PR #93): `confirm_lock` itself owns one
  `ensure_ticking()` (structural — future confirm paths inherit it);
  parked frames deliver post-confirm, pixel-verified blank.
- [Cursor frames while VT-paused](./resolved/cursor-frame-callback-when-paused-done.md)
  — RESOLVED 2026-09-17: `render()` (and the frame-callback dispatch) is
  skipped while the `--tty` session holds no DRM master, so a paused
  session neither renders frames `present()` would drop nor wakes clients
  to paint them; reactivation still mode-sets and repaints fully.
- [`wl_surface.offset` doesn't move the cursor hotspot](./rendering/cursor-surface-offset-hotspot.md)
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
- [A global fd/buffer ceiling across Wayland connections](./security/wayland-global-fd-ceiling.md) — the verdict's deferred half (low): per-connection bounds still multiply, but a shared ceiling would kill innocents for others' greed and needs its own refusal-form design first

### Packaging / tooling
- [Nix `src = self` invalidates the build on doc-only edits](./packaging/nix-src-fileset.md)
- [`x86_64-darwin` in `systems` breaks `flake check --all-systems`](./packaging/flake-x86-darwin-system.md)
- [`flake.nix` description drift](./packaging/flake-description-drift.md) (nit)
- [`smoke-test.sh` hardcodes temp paths](./resolved/smoke-test-temp-prefix-done.md) — RESOLVED 2026-09-17: every temp path derives from `$SMOKE_PREFIX` (unset = byte-identical legacy defaults); two concurrent runs with different prefixes proven green
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
