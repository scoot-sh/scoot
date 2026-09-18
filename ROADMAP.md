# flexwm roadmap

The ordered list of milestones, worked through with the cycle in
`CLAUDE.md`. This file is the **index**: the live ordered work and a status
table. The detail lives next to it, one file per item, so neither has to be
scrolled through to find the other:

- [`docs/roadmap/`](docs/roadmap/) — the 18 shipped milestones (plus 5a),
  one file each, verbatim review findings and hardware evidence included.
- [`docs/backlog/`](docs/backlog/) — everything unscheduled, split by area,
  with resolve history under `docs/backlog/resolved/`.

Entries carry YAML frontmatter so the sets are filterable without parsing
prose: `rg -l 'status: "open"' docs/backlog`, `rg -l 'priority: "high"'
docs/backlog`, `rg -l 'area: "protocols"' docs/roadmap`.

## Milestone status

| # | Milestone | Status |
| - | --------- | ------ |
| 1 | [Nested backend](docs/roadmap/01-nested-backend.md) | done |
| 2 | [Keybindings layer](docs/roadmap/02-keybindings-layer.md) | done |
| 3 | [Real tty/DRM backend](docs/roadmap/03-tty-drm-backend.md) | done |
| 4a | [Config file](docs/roadmap/04a-config-file.md) | done |
| 4b | [Window decorations](docs/roadmap/04b-decorations.md) | done |
| 5 | [Cursor rendering for `--tty`](docs/roadmap/05-cursor-rendering.md) | done |
| 5b | [VT-switch-back `EPERM`](docs/roadmap/05b-vt-switch-eperm.md) | done |
| 6 | [Real GPU rendering pipeline](docs/roadmap/06-gpu-pipeline.md) | **planned** |
| 7–18 | [Backlog-driven hardening and protocol work](docs/roadmap/) | done |

Item 6 (a GLES/Vulkan renderer as an optional alternative to pixman, selected
per-backend, with GPU-free operation kept as a hard requirement) is the only
remaining item on the original ordered list. Everything since item 7 has been
pulled forward from [`docs/backlog/`](docs/backlog/) rather than the order
above, at the user's direction or because a live crash-DoS or daily-driver
gap jumped the queue — each item's own file records why it landed when it did.

## Recently shipped (since 2026-09-15)

- **[`ext-idle-notify-v1` + `idle-inhibit-unstable-v1`](docs/backlog/resolved/ext-idle-notify-resolved.md)**
  (PR #38, 2026-09-15) — the automatic trigger session-lock had no other way
  to get. A `swayidle`-style daemon can now idle, resume and re-idle the
  seat (field-proven live); inhibitors hold it awake.
- **[The IPC bundle](docs/backlog/resolved/protocol-bundle-resolved.md)**
  (PR #39, 2026-09-15) — `OutputSnapshot.usable`, a focus-workspace-N action
  (`focus-workspace-index N`), and an ambient `locked` flag on every `Ok`
  reply. Landed with **no** `PROTOCOL_VERSION` bump: all three are
  defaulted/additive.
- **[The four protocols `foot` warned about](docs/backlog/resolved/foot-protocol-warnings-done.md)**
  (issue #40, PR #41, 2026-09-15) — `wp-cursor-shape-v1` (ten
  procedurally-drawn fallback shapes, plus real xcursor-theme loading so a
  themed session keeps its real artwork instead of regressing to line art),
  `xdg-activation-v1`, `xdg-toplevel-icon-v1`, and `text-input-v3` +
  `input-method-v2` (the IME popup). Verified before/after against `foot`'s
  own warnings.
- **[`xdg-activation-v1` had no input-serial gate](docs/backlog/resolved/activation-serial-validation-done.md)**
  (PR #42, 2026-09-16) — found by independent review of PR #41. An
  unfocused client with no user interaction at all could self-activate its
  own surface; a token now has to name a real, recent key/button event
  actually delivered to the requesting client, not just be young and under
  the count cap.
- **[`--tty` background "not painted"](docs/backlog/resolved/tty-background-not-painted-done.md)**
  (PR #43, 2026-09-16) — misdiagnosis, not a rendering bug: the background
  was always painted, and the smoke test's sample pixel sat on the cursor
  (the one thing only `--tty` draws). Fixed in the test script; no
  compositor code changed.
- **[Popup input](docs/backlog/resolved/xdg-popup-input-resolved.md)**
  (PR #44, 2026-09-16) — `xdg_popup.grab` is honoured, with a stated focus
  precedence (lock > exclusive layer surface > popup grab > window /
  click-focused layer surface): locking dismisses any open popup grab and
  refuses new ones, so a menu left open at lock time cannot receive the
  password. Layer-parented popups turned out to already work.
- **[Shared test harness + `cargo-nextest`](docs/backlog/resolved/large-test-file-organization-done.md)**
  (PR #45, 2026-09-16) — extracted the real-client harness five of the
  largest test files each reimplemented (707 fewer duplicated lines), split
  the two biggest by concern, adopted `cargo-nextest` alongside `cargo
  test`. Infrastructure, not a protocol or user-facing change.
- **[`ext-foreign-toplevel-list-v1`](docs/backlog/resolved/foreign-toplevel-list-done.md)**
  (PR #47, 2026-09-16) — the window list an external taskbar, dock or
  alt-tab switcher reads, alongside the workspace list `ext-workspace-v1`
  already gave them. Enumeration only (the protocol has no control
  requests); the identifier is `<generation>-<window id>`, so a client can
  go from a toplevel it found here to `flexwm msg action focus-window-id
  N`. Measured caveat, filed as its own item and
  [since resolved](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md):
  quickshell — and so DMS and Noctalia — binds the *wlr* protocol and
  ignores this one.
- **[Output management, read half](docs/backlog/resolved/output-management-read-only-done.md)**
  (PR #49, 2026-09-16) — `zwlr_output_manager_v1` (version 4), what a shell's
  Settings → Display page and `wlr-randr` read the screen's modes, position,
  scale and transform from. The wlr protocol rather than an `ext-` one only
  because no successor exists at the pinned rev — checked, not assumed — and
  Smithay carries no helper for either, so the handler layer is hand-written
  against the generated wlr bindings, the shape `gamma_control.rs` already
  uses. Every value is read from the same `Output` that configures
  `wl_output`, with a test that binds both on one connection and compares
  them. **Read-only by design**: `apply`/`test` always answer `failed`, and
  reconfiguration is
  [closed as a deliberate refusal](docs/backlog/resolved/output-management-reconfiguration-done.md)
  — flexwm has one output with a fixed mode, position and scale, so a
  `succeeded` that changed nothing would be a settings page that lies.
- **[`wlr-foreign-toplevel-management-unstable-v1`](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md)**
  (PR #50, 2026-09-16) — the other half of PR #47 above, and the window
  list DMS and Noctalia actually read. Published alongside the `ext-` one
  rather than instead of it, from the same three window-lifecycle events, so
  the two are one list described twice. Enumeration (title, app id,
  `output_enter`), `activate` and `close`, and `activated` as the only state
  bit flexwm can honestly answer — minimize/maximize/fullscreen are accepted
  and ignored, because the core has no concept of any of them and deciding
  what they mean in a scrolling-column layout is layout design, not wire
  format. No Smithay support at the pinned rev, so hand-rolled against the
  generated wlr bindings like `output_management.rs`. Verified live against
  the real quickshell: the window list populates, a panel click focuses the
  right window, and `close` closes it.
- **[`--tty` follows DRM hotplug](docs/backlog/resolved/tty-drm-hotplug-done.md)**
  (issue #48, PR #51, 2026-09-16) — `--tty` no longer mode-sets once and
  ignores the display afterwards: a udev monitor re-runs the connector/mode
  choice on every DRM `change` event, so unplugging the connector it is on
  falls back to another instead of going black until restart, and a VM host
  resizing or rescaling its window is followed rather than scaled. Still one
  output, deliberately: a hotplug re-runs the *same* single-connector choice
  startup makes, it does not start driving a second screen. The mode is no
  longer fixed for the process's life, which narrows (but, per the
  [reconfiguration record](docs/backlog/resolved/output-management-reconfiguration-done.md),
  does not close) the deliberate refusal of client-driven output changes.
  Two paths (a new mode list, and falling back to a *different* connector)
  could not be reproduced on the QEMU dev VM and still want confirmation on
  the vfkit/laptop hardware that filed the issue, which is why #48 is
  referenced rather than closed.
- **[Screen capture for clients](docs/backlog/resolved/screencopy-capture-done.md)**
  (PR #52, 2026-09-16) — `ext-image-copy-capture-v1` with
  `ext-image-capture-source-v1`, the standard path `grim`, a screen recorder
  and a shell's workspace-overview preview read the screen through. The
  `ext-` protocol and *not* `wlr-screencopy` alongside it, the opposite call
  to PR #50 and for the same reason — measured: `grim` 1.5.0 speaks only the
  `ext-` one, and quickshell 0.3.1 speaks it too. Unlike the three protocol
  items before it, Smithay implements this one, so flexwm writes handlers.
  **Output capture only**: a per-window source needs a second render target
  per session and was [its own
  item](docs/backlog/resolved/screencopy-toplevel-capture-done.md) — probed
  2026-09-17 and closed unreachable without building it (stock quickshell
  routes per-window thumbnails only to `hyprland-toplevel-export-v1`). A capture is
  parked and served from the frame tick rather than copied on request, which
  bounds it to one per session per frame and lets a repeat capture of an
  unchanged screen wait — both of which the protocol explicitly allows. The
  lock guarantee is inherited from `render()` rather than re-checked, plus
  one guard for the locked-but-not-yet-blanked window `flexwm msg screenshot`
  still has. Two upstream gaps found and worked around: Smithay never sweeps
  its own session list (an unbounded, client-driven leak) and never raises
  `duplicate_frame`. `flexwm msg screenshot` is unchanged.
- **[Activation / IPC focus leaves the keyboard on a clicked layer surface](docs/backlog/resolved/activation-clicked-layer-keyboard-done.md)**
  (PR #53, 2026-09-16) — the identical bug PR #50 fixed in its own `activate`,
  pre-existing in `xdg-activation-v1`'s `request_activation` and worse in
  `Request::Action` (no self-correcting unmap; the primary agent focus
  path). Both now spend the click; IPC spends it for the whole focus family
  and nothing else (pinned by a boundary test). Each new test confirmed to
  fail unfixed before being kept.
- **[`ext-workspace-v1` activation leaves the keyboard on a clicked layer surface](docs/backlog/resolved/ext-workspace-clicked-layer-keyboard-done.md)**
  (PR #54, 2026-09-16) — the client-protocol half PR #53 left out per scope,
  confirmed real rather than not-a-bug by two fail-first tests. The fix
  spends the click before `act` (refusing a locked session first), including
  on the already-active early return — which now runs the keyboard half, so
  the path agrees with IPC `FocusWorkspaceIndex` rather than differing by
  transport. The agent-facing IPC half was already fixed, so no agent loop
  silently mistypes; this closes the panel-that-stays-mapped exposure.
- **[Popup-grab keyboard holder over IPC](docs/backlog/resolved/popup-grab-focus-divergence-done.md)**
  (PR #55, 2026-09-16) — a window-focus change still does not dismiss an
  active popup grab (that deliberate guarantee stands), but `msg windows`
  now reports `popup_grab` per window, read off the held grab's root, so an
   agent can tell "window B is focused" from "B is focused but A's menu holds
   the keyboard". Additive field, no version bump; README's computer-use
   section states the agent rule and the asymmetric-`false` cases.
- **[Popup grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)**
  (PR #56, 2026-09-16) — filed from PR #44's own review: a grab now has to
  name a real, recent key/button/`enter` event delivered to the grabbing
  client (or continue its own open menu), so a background client can no
  longer take the keyboard on its own say-so. The `enter` half is the
  load-bearing subtlety — Qt passes its last-seen serial, which for a
  hover-opened menu is an enter — and the session half (nested submenus and
  same-flush menu replacements reusing the opening serial past the window)
  was measured live against real Qt and GTK menus, both proven still
  taking the keyboard. `start_drag` needed its own analysis and has since
  landed as [its own item](docs/backlog/resolved/dnd-grab-serial-validation-done.md).
- **[Screenshot encode off the event loop](docs/backlog/resolved/screenshot-encode-off-thread-resolved.md)**
  (PR #57, 2026-09-17) — the PNG encode (plus swizzle and reply framing)
  moved to a single FIFO worker; render and read-back stay on-loop. Loop
  stall per capture ~12ms → ~2ms derived, total latency unchanged by
  design. New, disclosed semantics: per-connection ordering via
  refused-with-retry, four captures max globally, `wait-idle` untouched.
- **[Toplevel screen capture, closed unreachable](docs/backlog/resolved/screencopy-toplevel-capture-done.md)**
  (PR #58, 2026-09-17) — phase-1 probe, no build: stock quickshell 0.3.1
  routes a `Toplevel` capture source exclusively to
  `hyprland-toplevel-export-v1` (version-exact source + binary inventory +
  live wire evidence), so the ext toplevel-source manager would never be
  bound. Fallback filed as shell thumbnails without a toplevel protocol
  (below). A second, wider gate found by the same probe: quickshell never
  instantiates *any* capture manager without `linux_dmabuf` feedback.
- **[Dmabuf-readiness probe: YES](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md)**
  (PR #59, 2026-09-17) — measurement only, zero executable change: a
  minimal `zwp_linux_dmabuf_v1` advertisement flips quickshell's readiness
  flag and the shipped ext output-capture path displays over shm. Honesty
  verdict independently re-derived from the protocol XML (answering
  `failed` is the designed fallback, not a violation); full blast-radius
  matrix deferred to the follow-up's acceptance, measured not reasoned.
- **[Minimal, honest `zwp_linux_dmabuf_v1`](docs/backlog/resolved/linux-dmabuf-advertisement-done.md)**
  (PR #60, 2026-09-17) — the follow-up the dmabuf-readiness probe gated:
  default feedback with the real scanout `dev_t` (`0` where no DRM node
  exists, logged once) plus the two LINEAR formats the shm pipeline serves,
  and imports answered `failed` — the protocol's own non-fatal fallback,
  which is the only truthful answer a pixman/shm compositor has. Measured
  on headless, no-node, `--nested` and `--tty`: quickshell's overview
  `ScreencopyView` displays over shm in all four. A genuinely
  dmabuf-allocating third-party client could not run on the GPU-less dev VM,
  so that half of the matrix is wire-test proven, not live — recorded as an
  environment limit, with GPU hardware as the scenario that would revisit it.
- **[`flexwm msg` EPIPE panic](docs/backlog/resolved/msg-client-broken-pipe-done.md)**
  (PR #62, 2026-09-17) — `msg ... | head` died with exit 101
  (`println!` panics on EPIPE; Rust ignores SIGPIPE). New `output.rs` maps
  a closed stdout to a quiet exit 0 at all six client-binary stdio sites
  (reply, screenshot bytes/summary, warning, `--help`, error report) —
  explicit per-write handling rather than a process-wide SIGPIPE
  disposition, which would have handed the co-resident compositor a
  crash-on-disconnect. A dead *socket* peer still exits 1. Fail-first
  integration test (fake server, ~1 MiB reply, reader closed); no protocol,
  IPC, or compositor changes, no README change (the error-response
  contract is untouched).
- **[`wl_data_device.start_drag` serial validation](docs/backlog/resolved/dnd-grab-serial-validation-done.md)**
  — the DnD half the popup gate (PR #56) deliberately left open. A drag now
  has to name the live implicit grab's press serial as delivered to the
  data source's own client (strict `contains`, not the popup's looser
  `contains_seen` — an `enter` can only be a live grab's serial while a
  popup grab holds the seat), so a background client can no longer convert
  someone else's held button into its own drag. Proven live against a real
  GTK drag source (`button(94)` → `start_drag(..., 94)`), with fail-first
  harness tests pinning the refusal and the dispatch floor beneath it. No
  README change (a refused drag is silent by protocol; the only
  legitimate-hit shape is a press held past the 10s window).
- **[An IME keyboard grab makes the activation gate credit a client that
  received nothing](docs/backlog/resolved/interaction-serial-ime-grab-done.md)**
  — RESOLVED 2026-09-17 as decide + pin, no behavior change: crediting the
  focused window is the intended outcome (the user is typing into it; the
  keystrokes arrive as composed text), and each of the ticket's three options
  was worse than the status quo. Pinned by two harness tests driving a real
  `grab_keyboard` (token creation for the focused window, refusal for the
  IME, creation + redemption end to end), each confirmed to fail with the
  not-recording behavior temporarily in place. No README change (no
  user-facing behavior changes).
- **[`spawn` hands its child no `XDG_ACTIVATION_TOKEN`](docs/backlog/resolved/activation-token-for-spawned-children-done.md)**
  (PR #65, 2026-09-17) — `State::spawn` (every keybinding and IPC `spawn`)
  mints via `create_external_token` and sets it on the child's `Command`,
  under both existing bounds: 30s freshness stamped at spawn, one of the
  same 64 slots swept the same way, and a full table meaning no token rather
  than an eviction (so an agent loop spawning apps cannot break interactive
  tokens). Inherited values are removed first; a spawn that never starts
  pulls its token back out. Seven harness tests around a real spawned child
  (env presence, slow-cold-start redeem, single-use, cap, sweep, failure,
  lock refusal), each behavior-changing one confirmed to fail unfixed; the
  smoke test asserts a live `foot`'s environ carries the token.
- **[IPC focus actions run a full `apply` even when nothing moves](docs/backlog/resolved/focus-action-no-op-fast-path-done.md)**
  (PR #67, 2026-09-17) — the fast path PR #54 gave `ext-workspace-v1`,
  mirrored per IPC focus variant: an already-there action spends the
  clicked-layer click and runs only the keyboard half instead of `act`.
  `FocusColumn`/`FocusWindow` steps deliberately stay on the full path
  (their no-op-ness needs column/stack positions the core doesn't expose).
  Measured live on the dev VM: sequential no-op latency ~1.6–1.8x better,
  flood throughput ~20k to ~38k req/s. No README change (purely internal
  latency, same non-difference PR #54 accepted).
- **[`ext-workspace-v1`'s cross-client check took two backend locks per
  manager, per `wl_output` bind](docs/backlog/resolved/ext-workspace-client-lookup-per-bind-done.md)**
  (PR #68, 2026-09-17) — filed from PR #50's review, fixed exactly as
  filed: the filter is now `ObjectId::same_client_as` (a lock-free field
  comparison, and the exact predicate the wayland-backend panic tests),
  copying `wlr_toplevel_output_bound`'s shape including its comment. Pinned
  by a cross-client test proven fail-first (neutered filter panics in
  `wayland-backend`, not a failed assertion). Measured ~10x on the
  predicate but ~300ns absolute — noise at session scale, so merged on
  correctness-clarity grounds, not a performance claim. No README change
  (same events, same order, same clients).
- **[A large `msg type` blocks the event
  loop](docs/backlog/resolved/msg-type-blocks-event-loop-resolved.md)** —
  RESOLVED 2026-09-17: `type` text capped at 16,384 characters per request,
  refused rather than delayed (the screenshot-rate-limit shape), sized by
  measurement on the dev VM (release `--headless`: ~2us/char plain,
  ~4.3us/char shifted, so the worst case costs ~75ms — comfortably
  sub-second). Counted in characters, not bytes; the other injected-input
  requests were audited and need no cap (`key` is one combo, `spawn` is one
  process). README's refusal list carries the new bound.
- **[Unbounded screencopy sessions and frames](docs/backlog/resolved/screencopy-session-cap-done.md)**
  (PR #70, 2026-09-17) — split as filed. Frames per session are capped: a
  `create_frame` past 16 live frames per client is refused pre-delegation
  with the protocol's own `duplicate_frame` error (per client, not per
  session — the pinned Smithay rev exposes no session identity
  pre-delegation — and a protocol error, not a silent ignore, which would
  leave an uninitialized object that panics the compositor). Sessions per
  client are closed as won't-fix on the per-surface precedent, with the
  residual cross-client walk latency stated honestly; they join the shared
  per-client accounting whenever the sibling entries land it. Fail-first
  harness tests (flood refusal, innocent-second-client, cycling drain,
  frames-outliving-session), ~1us added per `create_frame` measured noise,
   live `grim` unaffected. README's capture section carries the new bound.
- **[Screen capture's `Xrgb8888` alpha forcing, now
  conditional](docs/backlog/resolved/screencopy-xrgb-alpha-forcing-done.md)**
  (PR #71, 2026-09-17) — the ticket PR #52's review filed rather than decided
  mid-review: the `u128` opacity pass was most of what an `Xrgb8888` capture
  cost. Resolved as the ticket's third option — force the fourth byte only
  while `[appearance] background_color`'s alpha is below 1.0 (exact `< 1.0`,
  no epsilon; read per frame tick, not cached), which costs nothing in the
  default opaque configuration and keeps the guarantee where it was needed.
  Pinned byte-identical over opaque backgrounds, proven still-forcing over
  translucent ones. Measured on the dev VM: ~4.7% off a release `grim`
  capture in the default configuration. `Argb8888` untouched.
- **[An `exclusive` layer surface's own popup
  grab](docs/backlog/resolved/popup-grab-exclusive-self-dismiss-done.md)**
  — filed from PR #44's review: a launcher's own dropdown flashed open
  and instantly closed, refused and pre-empted by the very surface that
  opened it. Both checks now compare against the grab's root (the grant
  site reuses `find_popup_root_surface`; the pre-emption site reads the
  held grab's start-data focus the way PR #55 does), so a *different*
  exclusive surface still wins while the root itself never outranks its
  own menu. Fail-first harness tests for grant, pre-emption (incl. a
  nested submenu off the same root) and both unmap edges (no resurrection
  once dismissed, the root's own unmap leaving its menu up); README's
  rule-3 caveat removed now the claim is true as written. No hot-path
  benchmark (per-grab path plus one `Option` compare per focus
  derivation).
- **[Nothing bounded how many manager/list objects one client may
  bind](docs/backlog/resolved/ext-workspace-object-binding-cap-done.md)** —
  one shared per-client budget (8 binds across `ext_workspace_manager_v1`,
  `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1` and
  `zwlr_output_manager_v1`), sized at 2x the one-per-global legitimate max
  against the worst multiplier (self-created windows), refused with each
  global's own `finished` rather than a protocol error, released
  idempotently across `stop`/destroy/disconnect. Two findings on the way:
  a destructor `finished` sent inside `bind` panics wayland-backend's bind
  epilogue (so refusals defer to loop idle), and Smithay's
  `ForeignToplevelListState` exposes no bind seam — the ext list is now
  flexwm-owned, mirroring its wlr twin, with byte-identical wire behavior
  pinned by the existing suite. Fail-first harness tests per global plus
  isolation, drain, floor and same-batch stop/destroy shapes; bind-storm
  before/after (64 binds × 50 windows: 64 registered before, 8 after) and
  live quickshell (1 bind per connection, window list unaffected). README
  carries the per-global refusal semantics.
- **[No upper bound on *total* shm reservation per
  client](docs/backlog/resolved/shm-pool-count-cap-done.md)** — half
  landed, half proven unimplementable at the pinned rev. The byte total the
  ticket asked for cannot be built: the new pool's id is sealed inside
  `New<WlShmPool>`, `ShmPoolUserData` exposes no size, `Pool` is
  unexported, and no server API enumerates a client's objects (all
  re-verified in source) — closed as NEEDS-UPSTREAM behind a size
  accessor. What landed instead is a per-client *live-pool count* (128,
  claimed at `create_pool`, released on destroy/disconnect): it caps
  per-connection fds and mappings (one fd minimum per live pool; ~1000
  pools exhaust a 1024-fd `RLIMIT_NOFILE` for everyone), not bytes. Sized
  from wire measurement (`foot` 2×512 MiB arenas, Qt 2×~4 MiB, idle shell
  0; ~40 reasoned for a heavy multi-window app), refused with
  `InvalidStride` like the per-pool cap. Eight fail-first harness tests
  (flood, headroom, isolation, composition ×2, bad-fd, drain, resize-pin --
  the bad-fd one pins review's deterministic leak: a probe mapping the exact
  call Smithay is about to make now gates the claim); one
  harness race found and fixed (pools read vs disconnect cleanup).
  Residual filed as its own item: Wayland connections are unbounded, so
  per-connection bounds multiply ([Wayland connection
   cap](docs/backlog/resolved/wayland-connection-cap-done.md)). README carries
  the new bound.
- **[The first click on a fresh lock screen, before the mouse has moved,
  reaches nobody](docs/backlog/resolved/session-lock-first-click-done.md)**
  (PR #76) — fixed as filed: pointer focus is re-derived on the commit
  that maps a lock surface (`new_surface`'s own derivation runs while it
  is still unmapped, so the hit test found nothing and `wl_pointer.enter`
  waited for the first mouse move, +4975ms on real `--tty`). Recognition
  is one branch per non-window, non-layer commit while unlocked plus one
  typemap probe while locked; zero new state. Four harness tests (two
  fail-first, one pinning the already-working unlock symmetry, one pinning
  no focus before mapped); window-commit throughput unchanged
  (before/after ranges overlapping). No live `--tty` re-measurement —
  stated as an environment call in the resolved record, with harness wire
  evidence instead. README's not-guaranteed bullet removed.
- **[An IME popup over a lock screen](docs/backlog/resolved/ime-popup-over-lock-screen-done.md)**
  — the locked render path gathers each current lock surface's popup tree
  and sends it frame callbacks, so a passphrase that needs an IME gets its
  candidate window at the caret. The trust call is recorded, not implicit:
  an IME is trusted with pixels for the focused field's candidate window
  (composition already routes every keystroke through it), while background
  xdg and IME popups stay hidden and callback-starved — the PR #44 password
  guarantee through the new element source, pinned by exclusion tests
  alongside the inclusion one.
- **[An already-bound `wl_output` client is never told a `--nested` resize's
  new mode is
  preferred](docs/backlog/resolved/wl-output-preferred-flag-on-late-mode-done.md)**
  — fixed as filed: `set_mode` marks the new mode preferred *before*
  `change_current_state` sends it (the ticket's batching alternative checked
  against the pinned rev and ruled out — the send is synchronous, so call
  order is what the wire sees). One fix covers `--nested` and `--tty`
  (both reach `set_mode` through `resize_output`); pinned by a fail-first
  harness test asserting the `wl_output` and `wlr-output-management`
  resize batches agree. Live `--nested` steady state confirmed under cage;
  the transient itself is harness wire evidence (external clients cannot
  win the bind-before-configure race — stated, not papered over). No
  README change (no probed client reads the bit).
- **[`[binds]` capital
  letters](docs/backlog/resolved/binds-capital-letter-done.md)** — a
  `[binds]` entry naming a capital letter (`"A" = "close"`) now binds the
  unshifted key instead of parsing, loading, and never firing. The fold
  already existed whole-name; this scopes it to single ASCII letters only
  (whole-name silently redirected `"OE"`/Œ to `"oe"`/œ — measured), warns
  naming the bind (`"A"` means plain `a`, not `shift+a`), and leaves
  `keysym_named` untouched so `flexwm msg key A` keeps refusing. Fail-first
   unit tests per direction plus a permanent smoke-test section (live
   `--headless`: warn logged, injected `a` closes the window).
- **[`--width`/`--height`
  bounds](docs/backlog/resolved/width-height-bounded-done.md)**
  (PR #82, 2026-09-17) — refused
  past 65535 per axis at parse (the most DRM itself can report for a mode
  axis, with room to spare past 16K hardware), which closes the chain that
  made item 12(b)'s output-derived `min_size` limit vacuous at ~2×10⁹
  (worst case now 131070 a side at the output-scale floor, ~15000x inside
  `i32`). `Rect::inset`/`right()`/`bottom()`, `scroll_into_view` and the
  arrange on-screen test saturate instead of overflowing, each pinned by a
  fail-first test; window-count-derived products audited and left under the
  existing `MAX_GAP` disclosure.
- **[`locked` waits for vblank confirmation, not just a
  render](docs/backlog/resolved/session-lock-vblank-confirm-done.md)** —
  resolved 2026-09-17 (PR #84): under `--tty` `locked` waits for the vblank of the
  flip carrying the blanked frame (flip-sequence tracked, so a lock raced
  with an in-flight flip confirms on the next one), with a one-second
  fallback confirming anyway rather than hanging a switched-away locker;
  headless/nested confirm on render unchanged. Verified live on the dev
  VM (completions fire; three consecutive locks each vblank-confirmed).
  Found alongside, pre-existing and filed separately: a lock surface
  mapped after the confirming frame never appears on live `--tty`
  ([surface-not-drawn-live](docs/backlog/resolved/session-lock-surface-not-drawn-live-done.md))
   — CLOSED UNREPRODUCED 2026-09-17 (six green live sessions + mechanism
   audit; likely stale-socket artifact, VM swept).
- **[`scripts/smoke-test.sh` hardcoded temp
  paths](docs/backlog/resolved/smoke-test-temp-prefix-done.md)** —
  RESOLVED 2026-09-17: every socket, log, screenshot, config and scratch
  path derives from `$SMOKE_PREFIX` (unset = byte-identical legacy
  defaults, so existing `SOCKET`/`LOG` callers see no change); two
   concurrent prefixed runs proven green with fully disjoint file sets.
- **[The pointer starts at the output's origin, not
  centred](docs/backlog/resolved/pointer-starts-at-origin-done.md)** —
  RESOLVED 2026-09-17: `State::place_pointer_at_output_centre`, called
  once from `headless::init_named` so all three backends place it at the
  same init point, through `pointer_move_quietly` (no idle-timer reset,
  no focus derived, no interaction serial minted). Logical extent, so a
  scale other than 1 centres correctly; resize and VT-switch
  reactivation deliberately leave the pointer alone. Live `--tty`
  screenshot proves the cursor bitmap at the output centre; the smoke
  test's park-the-pointer workaround stays.
- **[Cursor frames while
  VT-paused](docs/backlog/resolved/cursor-frame-callback-when-paused-done.md)**
  — RESOLVED 2026-09-17: `render()` and the frame-callback loops are gated
  on DRM master, so a VT-paused `--tty` session renders nothing and wakes
  no clients (paused burn under a 600-motion driver: 10–12 → 7–8 jiffies
  release, 31–39 → 21–24 debug; frozen-framebuffer A/B proves no render
  runs while paused). Reactivation still mode-sets and repaints
  byte-identically; lock-while-paused and grab/IME-across-pause are
  construction-verified (no live clients for either on the dev VM).
- **[`single-pixel-buffer-v1`](docs/backlog/resolved/single-pixel-buffer-done.md)**
  — RESOLVED 2026-09-17: `wp_single_pixel_buffer_manager_v1` advertised
  (Smithay carries the whole protocol at the pinned rev, so this is three
  lines of state plus docs and tests, no hand-rolled handler). Five
  fail-first harness tests around a real client (RGBA read-back, 1x1
  dimensions, viewport-scaled render pinned pixel-for-pixel, manager
  destroy, attached-buffer destroy, zero shm-pool budget claimed); real
  `foot` binds the global live.
- **[`relative-pointer-unstable-v1` + pointer
  constraints](docs/backlog/resolved/relative-pointer-done.md)** —
  RESOLVED 2026-09-17: `zwp_relative_pointer_manager_v1` advertised
  (Smithay carries both halves at the pinned rev, same three-lines-plus
  shape), with the motion core feeding unclipped deltas on every focused
  move. Two corrections to the bundle entry's assumptions, both verified
  in source: the constraints global was never advertised (only the trait
  bound existed), and relative events are focus-gated per the protocol,
  not lock-gated. Sixteen fail-first harness tests pin exact vectors
  (lock hold/resume, confine hold/resume, pre-accel pairs, edge
  unclipping, per-client streams, lock-before-focus engagement, regional
  per-axis clamp and gating, session-lock round-trip freeze-through); live
  `wayland-info` + `foot` prove the advertisement, `foot` binds neither
  (it uses neither protocol). Motion-path benchmark: release unfocused
  noise, focused +~190ns/event (~1.2% of one 16ms frame per second at
   1000Hz).
- **[`presentation-time` (`wp_presentation`)](docs/backlog/resolved/presentation-time-done.md)**
  — the last child of the general-gaps bundle: precise frame-timing
  feedback for smooth video/animation clients (Smithay carries the whole
  protocol at the pinned rev, same three-lines-plus shape). Each presented
  frame is stamped with its backend's honest handoff time
  (render-complete on headless, host-commit on nested, flip-issue on tty),
  only displayed surfaces are stamped, and a rendered-but-dropped frame
  stamps nothing. Seven fail-first harness tests (sane fields, monotonic
  timestamps, supersede-discarded, unmapped/locked absence, disconnect);
  live `foot` proves the advertisement. The bundle
  (`docs/backlog/protocols/protocol-gaps-general.md`) is now fully done.
- **[A held pointer lock across a session
  lock](docs/backlog/resolved/pointer-lock-session-lock-done.md)** (PR #92,
  2026-09-17) — the password-leak-class regression PR #89 introduced: the
  lock transition's focus refresh is a zero-delta move, which a held
  constraint resolves to `Held`, so deltas, buttons and axis kept streaming
  to the game while the lock surface never got `enter` (and the cited
  chord recovery never runs while locked). The transition now deactivates
  the focus surface's held constraint first — the game sees `unlocked`,
  the persistent entry re-arms on unlock with no new request — and the test
  that pinned the freeze as intended is rewritten as an explicit behavior
  correction. Confinement needed no fix (fail-open + leave-deactivation;
  pinned), and the adjacent per-event region clone is gone (two short
  borrows; overlapping bench ranges, structurally allocation-free).
- **[A screencopy frame parked for a session lock's blank was never re-armed
  once the vblank
  confirms](docs/backlog/resolved/screencopy-parked-across-lock-confirm-done.md)**
  (PR #93, 2026-09-17) — the regression PR #84 introduced moving lock
  confirmation onto the DRM vblank: the blank tick drops the frame timer
  (a parked capture is none of its re-arm conditions) and neither confirm
  path restarted it, so the parked frame sat undelivered until some later
  commit re-armed the ticker — forever, for a lock-and-never-commit locker.
  Fixed as one `ensure_ticking()` in each confirming branch (the parked-in-
  the-re-arm-set alternative would pin the timer at 60Hz for every static-
  desktop preview); five fail-first harness tests pin the re-arm on both
  paths, the one-tick steady-state cost, multi-session delivery and the
  abandoned-locker shape. No README change (bug fix within protocol-
  permitted behavior, no user-facing surface).
- **[Noctalia re-probe: gap 1 closed in the
  field](docs/backlog/resolved/noctalia-reprobe-done.md)** (2026-09-18,
  probe + docs, no code) — the P0 lock-teardown kill the 09-14 probe
  field-confirmed now survives two full lock → PAM auth →
  `unlock_and_destroy` cycles against the real quickshell client (pid
  unchanged, live screenshots, zero kill-signature lines; the exact
  fatal destroy + null-commit sequence is on the wire with the
  connection surviving). Dismissal sweep (12 + Escape), toast, foot,
  workspace pill all live; gaps 2–8 re-check as resolved, upstream, or
  deliberate, so the ticket is resolved with nothing new filed.
- **[DMS re-probe: gap 1 stays closed in the
  field](docs/backlog/resolved/dms-reprobe-done.md)** (2026-09-18,
  probe + docs, no code) — the sibling re-probe: the exact spotlight
  teardown plus two full lock → PAM auth → `unlock_and_destroy`
  cycles against the real quickshell client (pid unchanged throughout,
  live screenshots after every step, zero kill-signature lines; both
  fatal sequences are on the wire with the connection surviving, so
  DMS's unlock path does not trip the lock-role signature Noctalia
  died on). Dismissal sweep (spotlight, clipboard, notifications
  modal, dash), toast, DankDash, two `foot` windows (one IPC-spawned,
  one DMS-launched and focused — gap 8's launch question answered);
  gaps 2–8 re-check as resolved, upstream, deliberate, or shell-side
  presentation, and the 512 live-buffer bound is untouched, so the
  ticket is resolved with nothing new filed.
- **[The live-pool cap never bounded fds or
  mappings](docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md)**
  — RESOLVED 2026-09-17: the pool count's fd/mapping claims corrected
  everywhere (it bounds live pool objects + the address-space envelope),
  and the real fix landed as a per-client live-`wl_buffer` cap (512,
  uniform across shm/dmabuf/single-pixel factories, refused with a
  protocol error per interface, released on destroy/disconnect) that
  catches exactly the create-pool/create-buffer/destroy-pool bypass. Sized
  from wire measurement (`foot` holds 2 live buffers steady); the
   per-connection retained-fd bound is now 512 buffers + 128 pools, and
   `wayland-connection-cap`'s math is updated to say so.
- **[No cap on Wayland connection
  count](docs/backlog/resolved/wayland-connection-cap-done.md)** —
  RESOLVED 2026-09-18: verify-first re-derivation found the ticket
  undersold the bug — an `EMFILE` on the Wayland listener did not deny
  sockets, it exited the whole compositor (Smithay's source propagates
  `accept` errors out of `run()`; proven live pre-fix with prlimit).
  Fixed as a Wayland accept source that sheds like the IPC one (shared
  spare/classify primitives, EOF to the shed client, immediate recovery),
  fail-first suite plus live pre/post kill pair. The count itself is
  closed as an accepted tradeoff mirroring the IPC sibling (any usable
  count admits the two greedy connections that fill the 1024-fd table);
  a global fd/buffer ceiling is filed as its own low-priority follow-up
   since it would kill innocents for others' greed.
- **[An IME keyboard grab blocks every popup
  grab](docs/backlog/resolved/popup-grab-blocked-by-ime-grab-done.md)** —
  RESOLVED 2026-09-18: verify-first found the reverse order unhandled (an
  IME grabbing while a menu held the keyboard left it mapped with no
  keyboard, `popup_done` never sent). A live grab displaced by a foreign
  keyboard grab is now dismissed on the next dispatch, so either order ends
  with no menu while the IME holds the seat; precedence (lock > exclusive
  layer > IME grab > popup grab) pinned by two fail-first harness tests and
  documented in the README. The per-surface IME-scoping design stays open
  as filed (needs a Smithay-side design that does not exist).
- **[`xdg-toplevel-icon-v1` pixel-buffer
  icons](docs/backlog/resolved/toplevel-icon-buffers-done.md)** (PR #100,
  2026-09-18) — verify-first close, no behavior change. The
  ticket's three suspicions checked against the pinned sources all came
  back clean (the protocol leaves `release` unused by design, Smithay
  kills only the offending client with `NoBuffer`, icon buffers claim
  against the same 512 live-buffer budget as every other `wl_buffer`,
  and neither foreign-toplevel list protocol has an icon event to expose
   pixels through). Name-only IPC stands as the honest scope; four
  fail-first harness tests pin the buffer half (counting, kill
  containment, disconnect drain, cap fill).
- **[Lock blanks immediately instead of waiting for the first
  surface](docs/backlog/resolved/session-lock-blank-timing-done.md)** —
  RESOLVED 2026-09-18 (decide + pin, no behavior change): the accept →
  input-captured → first-frame-blanks → `locked`-after-blank ordering
  verified in-harness (the desktop-visible-while-input-locked window is
  the deliberate, secure direction; the niri-shape wait stays declined),
  pinned by a new combined fail-first test.
- **[Lock manager global offered to every
  client](docs/backlog/resolved/session-lock-global-restriction-done.md)** —
  CLOSED 2026-09-18 as an accepted tradeoff (no code): the protocol names
  no privilege mechanism, the pinned Smithay filter is hide-from-registry
  only, and even security-context support would not key a locker
  allow-list (it marks sandboxes, not lockers) — so the ticket's own
  `blocked:` line was wrong. Ordinary-client lock + second-client
  takeover stay pinned by the existing harness suite.
- **[Lock surfaces are per-output; flexwm has one
  output](docs/backlog/resolved/session-lock-per-output-done.md)** —
  RESOLVED 2026-09-18 (PR #103, pin + document, no behavior change): first blanked
  frame on the one output confirms whatever the surface count, every
  admitted surface shares its size (new two-surface suite: first on top,
  keyboard on first, resize reaching all), `OUTPUT_ID`'s doc lists the four
  sites multi-output must revisit. Duplicate-bind admission stays open as
  its own entry.
- **[One physical output can hold unboundedly many lock surfaces if the lock
  client binds `wl_output` more than
  once](docs/backlog/resolved/session-lock-duplicate-output-done.md)** —
  RESOLVED 2026-09-18 (refuse, no Smithay patch): the admission question PR
  #103 punted here. The protocol mandates `duplicate_output` per *output*,
  Smithay enforces it per *resource*, so flexwm refuses a second live
  surface for an already-covered output with code 3 on the lock, keyed on
  the physical `Output`; destroying the surface frees it for a rebuild.
   Neither probed shell ever holds two at once. Supersedes PR #103's
   two-surface composition pins; the single-surface per-output pins stand.
- **[A layer surface that commits but never attaches a buffer holds its
  exclusive zone](docs/backlog/resolved/layer-surface-bufferless-exclusive-zone-done.md)** —
  RESOLVED 2026-09-18 (decide + pin, no behavior change): the zone applies
  from the buffer-less initial commit the protocol mandates, not the first
  buffer — every healthy bar passes through that state, so windows never
  jump when the first buffer lands. Filtering would mean forking Smithay's
  `arrange` geometry (re-verified at the pinned rev: no mapped/buffer check).
  Three fail-first harness tests pin the edges (never-draws held until
  disconnect, buffer-less destroy, post-buffer zone drop); no timeout by
  design, lifetime bounded by disconnect/destroy.
- **[Niche protocol gaps, triaged per
  sub-item](docs/backlog/resolved/protocol-gaps-niche-done.md)** —
  RESOLVED 2026-09-18: all nine sub-items of the niche bundle get minimum
  honest dispositions. `wp_alpha_modifier_v1` and
  `wp_content_type_manager_v1` implemented (Smithay carries both; the
  factor blends end to end through pixman, the hint is stored and honestly
   ignored), each with fail-first harness tests; `tablet-v2` filed as its
   own entry ([tablet-v2](docs/backlog/input/tablet-v2.md)); screencopy,
   output-management and the VT-pause cursor item already resolved;
   security-context and the cursor `Vec` closed deliberate; cursor-hotspot
   offset closed needs-upstream (since overturned and fixed flexwm-side —
   see below).
- **[A `present()` skipped for an in-flight flip consumes that frame's
  damage](docs/backlog/resolved/present-skip-eats-frame-damage-done.md)**
  — RESOLVED 2026-09-18: verify-first found the filed shape self-heals
  (an unwritten skip's retry re-reads a larger age, so the damage history
  still covers it — pinned, not "fixed"), while the adjacent refused-flip
  shape loses twice (the freed slot keeps a fresh age so the retry reads
  age 1 and draws nothing, and no vblank is owed so no retry is even
  triggered). The commit-failure arm now clears the failed slot's age and
   arms a bounded timer retry (3 consecutive refusals, then quiet until
   genuine damage); the in-flight path is untouched. Fail-first unit tests
   plus a Smithay-contract pin; headless render throughput unchanged
   (overlapping before/after ranges).
- **[`wl_surface.offset` on a cursor surface moves the
  hotspot](docs/backlog/resolved/cursor-surface-offset-hotspot-done.md)**
  — RESOLVED 2026-09-18: PR #106's NEEDS-UPSTREAM triage re-derived and
  overturned — Smithay core never adjusts the hotspot, but Smithay's own
  anvil does the decrement compositor-side in its shell commit hook, so
  flexwm can too. `Cursor::note_surface_commit` (one call site at the end
  of `CompositorHandler::commit`) decrements by this commit's
  `buffer_delta`, saturating rather than wrapping (both operands are
  client-controlled `i32`) and gated on the active cursor surface. Four
  fail-first harness tests around a real client (offset + accumulation
  incl. negative hotspot, post-re-set, `i32::MIN` saturation, other-surface
  negative control); no README change (protocol fix, no user-facing
  surface).
- **[`Cursor::element`'s one-element fallback
  `Vec`](docs/backlog/resolved/cursor-element-per-frame-alloc-done.md)**
  — CLOSED DELIBERATE 2026-09-18 with measured numbers (PR #106's triage
  re-derived, not relayed): the fallback `vec![...]` is already the minimal
  allocation (448 bytes at capacity exactly 1, ~90–120ns net per call
  release on the dev VM), firing 0 times/sec at idle and at most ~62.5/sec
  during dirty `--tty` frames — ~7µs/s against a 16ms frame budget. The
  ticket's push-into-a-local-`Vec` shape allocates 4x the bytes (1792,
  measured: Rust's minimum non-zero capacity), and a persistent buffer's
  signature ripple costs more than it saves while `render()` keeps several
  per-frame `Vec`s regardless. Pinned by two capacity tests (fail-first
  proven against a wasteful variant); no compositor code changed, no README
  change (no user-facing surface).
- **[Parked-captures poll flake](docs/backlog/resolved/screencopy-parked-poll-flake-done.md)**
  — RESOLVED 2026-09-18 (PR #110, test-only, no production change): the filed
  mechanism was corrected by per-step serial instrumentation — the trip is
  `Ready` at an *unmoving* serial (a `delivered`-lag at park time, the
  pre-map frame consumed by an earlier tick), not an advance between park
  and poll, so serving it is correct behavior. Fixed with a quiescence wait
  plus a synchronize-then-assert retry (one retry proven max, bounded at
  three); the delivered-frames pin still fails with the re-arm neutered.
  Post-fix: 40/40 targeted + 12/12 full-binary under the oversubscription
  that tripped 6/40 + 1/12 pre-fix, full set green, smoke 17 ok. Found
  alongside and filed separately (low, load-only): an activation
  taskbar-click settle flake under the same abusive load.
## What's next

The backlog is the source of truth for what to pick up; this is the current
read of it, not a commitment. Both shell probes are now resolved
outright — [DMS gaps](docs/backlog/resolved/dms-reprobe-done.md)
(re-probed 2026-09-18: the spotlight teardown plus two full lock →
auth → unlock cycles survive in the field, DMS's unlock path doesn't
trip the lock-role signature, everything else re-checks as
resolved/upstream/deliberate/shell-side, nothing new filed) and the
[Noctalia probe](docs/backlog/resolved/noctalia-reprobe-done.md)
(re-probed 2026-09-18: the lock-teardown kill survives two full
lock → auth → unlock cycles in the field, everything else re-checks as
resolved/upstream/deliberate, nothing new filed). What's left of both
was already filed individually under `docs/backlog/protocols/` at
medium priority — the effective top of what's actually open.

1. **Confirm `--gpu` fixes the Asahi Linux `--tty` failure** — needs the
   user's own hardware, not the dev VM (no split GPU/display-controller
   topology there). Unblocks the
   [`[tty] gpu` config key](docs/backlog/tty/tty-gpu-config-key.md).
2. **Medium-priority protocol gaps**, mostly what's left of the DMS/Noctalia
   probes (see "Shell enablement" below for their recommended order):
    [screencopy's toplevel half](docs/backlog/resolved/screencopy-toplevel-capture-done.md)
    is CLOSED UNREACHABLE (phase-1 probe, no build — stock quickshell routes
    per-window thumbnails only to `hyprland-toplevel-export-v1`); the
    thumbnail fallback
    [shell thumbnails without a toplevel protocol](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md)
    is CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build — the overview
    preview lights up on shipped `main`, the screen-source + clip-crop
    recipe is proven live pixel-for-pixel, and DMS's `TileItem.qml`
    hard-requires a `Toplevel` source, so the shells must change; current
    Noctalia has no per-window live-thumbnail view at all).
   [Popup grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)
   is also done — a grab now has to name a real, recent key/button/enter
   event delivered to the grabbing client (or continue its own open menu).
   It was filed from PR #44's own review alongside
   [a window-focus change doesn't dismiss an active popup
   grab](docs/backlog/resolved/popup-grab-focus-divergence-done.md), which
   shipped as an IPC `popup_grab` field in PR #55.
3. **[IPC connection cap and the half-closed-connection
   leak](docs/backlog/resolved/ipc-connection-cap-resolved.md)** — RESOLVED
   2026-09-16: 64 concurrent connections, refused with a reason past that,
   a write-stall deadline that drops a peer which has stopped reading (the
   leak a cap alone could not free), and — found by the review of that — a
   cap on how long a `wait-idle` may park a connection, without which 64
   parked waiters from a since-crashed agent wedged the whole control
   channel. The third concern that entry bundled — [capture and encode on
   the event-loop
   thread](docs/backlog/resolved/screenshot-encode-off-thread-resolved.md) —
   is RESOLVED 2026-09-17: the PNG encode (plus swizzle and reply framing)
   moved to a single FIFO worker, with per-connection ordering held by
   refusing anything else on a connection with a capture in flight, a bound
   of four captures across the compositor, and `wait-idle` untouched. The
   first of the two adjacent things found while reviewing the cap work --
   [the accept loop and
   `EMFILE`](docs/backlog/resolved/accept-loop-emfile-resolved.md) -- is
   RESOLVED 2026-09-17: the loop matches on the error kind
   (`WouldBlock`/`Interrupted` end it quietly as before, anything else is
   loud), fd exhaustion sheds one pending connection per turn through a spare
   fd (no back-off, no added delay once pressure lifts), and a dead listener
   deregisters rather than spinning. The other -- [what a
   shared connection table costs an innocent
   client](docs/backlog/resolved/connection-cap-denies-the-same-user-done.md) -- is
   closed as an accepted tradeoff (no workload has hit it; kept as the landing spot).
4. Small, unblocked low-priority fixes: [`msg key` modifier
    resolution](docs/backlog/resolved/msg-key-modifier-resolution-done.md)
    (PR #83, resolved — modifiers resolve through the keymap probe like
    `type_text`; toggle-option layouts accept combos, `msg key A` still
    refuses),
   [`[binds]` capital
   letters](docs/backlog/resolved/binds-capital-letter-done.md) (resolved —
   single-ASCII-letter fold at parse time, warn on normalize, `msg key A`
   still refuses),
   [`--width/--height`
   bounds](docs/backlog/resolved/width-height-bounded-done.md) (resolved —
   refused past 65535 per axis at parse, layout siblings saturate).
5. [Rename `flexwm` → `flex`, split out
   `flexctl`](docs/backlog/meta/rename-flex-family.md) — decided, explicitly
   scheduled **last** in the burn-down, per the entry's own frontmatter.

## Shell enablement (DMS / Noctalia probes, 2026-09-14)

Two Quickshell shells were probed end to end
([DMS gaps](docs/backlog/resolved/dms-reprobe-done.md),
[Noctalia results](docs/backlog/resolved/noctalia-reprobe-done.md) —
both re-probed 2026-09-18 and resolved: every overlay dismissal and
lock → auth → unlock cycle survives in the field, everything else
re-checks as resolved/upstream/deliberate/shell-side) after
landing gamma-control, both data-controls and primary selection
(PR #32) plus two destroy-teardown kill fixes (PRs #34, #36). Both
shells render fully; Noctalia is the better target (generic
`ext-workspace-v1` backend, richer IPC). Remaining gaps, in the
probes' recommended order:

1. [Popup input](docs/backlog/resolved/xdg-popup-input-resolved.md)
   — RESOLVED 2026-09-16: `xdg_popup.grab` is honoured, so a menu takes
   the keyboard, Escape/arrow keys/typeahead reach it, and clicking
   outside dismisses it (the only thing that ever sends `popup_done`).
   The focus precedence it sits at is stated once in `popup.rs`: the
   session lock and an `exclusive` layer surface both pre-empt a grab
   (dismissing the menu rather than silently outranking it), and the
   grab wins over the focused window and over a click-focused
   `on_demand` layer surface — so a launcher stays typeable and a bar's
   own dropdown is not dismissed by the bar that opened it.
   Layer-parented popups turned out already to work; implementing the
   handler the entry asked for would have put two tree nodes on one
   surface (measured). Follow-up filed, and since resolved:
   [grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)
   — a grab now names a real, recent key/button/enter event or continues
   its own open menu, proven live against real Qt and GTK menus.
   Neither probed shell exercises any of this directly: DMS and Noctalia
   both route their own menus through layer surfaces, with zero
   `xdg_popup` wire traffic in either probe — the first real client for
   this work is an ordinary GTK/Qt toolkit menu, not DMS or Noctalia.
2. [Idle](docs/backlog/resolved/ext-idle-notify-resolved.md) — RESOLVED
   2026-09-15: auto-lock's trigger exists and is field-proven with real
   swayidle; what remains is the user's own daemon config, not compositor
   work.
3. [`foreign-toplevel`](docs/backlog/resolved/foreign-toplevel-list-done.md)
   — RESOLVED 2026-09-16, in two halves. The successor the entry told its
   reader to check for exists (`ext-foreign-toplevel-list-v1`, PR #47) and is
   implemented, so a standards-following taskbar or switcher can list windows
   and map each one back to its `flexwm msg windows` id — but it did *not*
   unlock these two shells: quickshell 0.3.1 is offered the global and never
   binds it (measured, wire-level), because its `ToplevelManager` is a wlr
   client. [`wlr-foreign-toplevel-management`](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md)
   (PR #50) closes that half — list, click-to-focus and close, all verified
   live against the real quickshell. Minimise/maximise remain no-ops, since
   flexwm's core has no concept of either.
4. [`output-management`](docs/backlog/resolved/output-management-read-only-done.md)
   — HALF-RESOLVED 2026-09-16 (PR #49): the query half a shell's display page
   binds is implemented (`wlr-output-management-unstable-v1` v4 — no `ext-`
   successor exists at the pinned rev, so the standing preference had nothing
   to prefer). Reconfiguration is deliberately refused and
   [closed as an accepted tradeoff](docs/backlog/resolved/output-management-reconfiguration-done.md)
   (2026-09-18, no code — no authorization concept in the protocol, per-backend
   honesty gaps, no shell demand): `failed` stays the honest answer.
5. [`screencopy / image-capture`](docs/backlog/resolved/screencopy-capture-done.md)
    — HALF-RESOLVED 2026-09-16 (PR #52): `ext-image-copy-capture-v1` with
    `ext-image-capture-source-v1` for **output** capture, which is the
    workspace-overview preview half. The per-window thumbnail half was
    [probed and closed unreachable](docs/backlog/resolved/screencopy-toplevel-capture-done.md)
    rather than built (stock quickshell 0.3.1 speaks only
    `hyprland-toplevel-export-v1` for a `Toplevel` source); its fallback,
    [shell thumbnails without a toplevel protocol](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md),
    is CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build): the overview
    preview lights up with real pixels on shipped `main` (PR #60's
    advertisement, re-driven), and the screen-source + clip-crop recipe for
    per-window thumbnails is proven live pixel-for-pixel — but DMS's
    `TileItem.qml` hard-requires a `Toplevel` source and current Noctalia
    has no per-window live-thumbnail view, so the change belongs upstream.
    IPC screenshots stay regardless.
6. DMS unlock-path re-probe — done 2026-09-14 (see the re-probe note
   in the DMS gaps entry): lock → auth → unlock teardown survives on
   current `main`, and so does a spotlight open/Escape-dismiss cycle.
   Both shells' destroy kills are now proven fixed, not inferred.
   Re-probed 2026-09-18 and resolved outright
   ([record](docs/backlog/resolved/dms-reprobe-done.md)): the exact
   spotlight teardown plus two full lock → auth → unlock cycles
   survive, DMS's unlock path doesn't trip the lock-role signature,
   gap 8's launch question is answered (DMS-spawned app maps
   focused), and "No compositor detected" stands recorded as
   upstream.

Small follow-ups already filed alongside:
[`gamma-control`](docs/backlog/resolved/gamma-control-followups-done.md)
(resolved, PR #78),
[`layer-destroy`](docs/backlog/resolved/layer-destroy-review-followup-done.md)
(resolved, PR #77).

Item 6 (the GPU pipeline) remains the one *ordered* milestone still open; it
has never been ahead of the daily-drivability and correctness work the backlog
keeps producing, and that trade can be revisited at any time.

## History

The pre-split `ROADMAP.md` was 4,314 lines: ~2,817 of milestone write-ups
(17 of 18 already done) in front of ~1,489 of backlog. It was split on
2026-09-14 so the live work is readable without scrolling past an archive.
No entry's prose was rewritten in the move — each file is the original
entry with only its list marker and continuation indent removed, plus
frontmatter. The split was verified by re-deriving every body from
`ROADMAP.md`'s git history and diffing: zero differences across all 21
milestone and 57 backlog files.
