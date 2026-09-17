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
  [its own deferred item](docs/backlog/protocols/output-management-reconfiguration.md)
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
  longer fixed for the process's life, which makes `--tty`'s `set_mode` the
  smallest remaining piece of
  [output-management reconfiguration](docs/backlog/protocols/output-management-reconfiguration.md).
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
  per session and is [its own
  item](docs/backlog/protocols/screencopy-toplevel-capture.md). A capture is
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

## What's next

The backlog is the source of truth for what to pick up; this is the current
read of it, not a commitment. The backlog's own "High priority" entries
([DMS gaps](docs/backlog/protocols/dms-enablement-gaps.md),
[Noctalia probe](docs/backlog/protocols/noctalia-probe.md)) are stale probe
reports now: their P0 findings are resolved elsewhere (see "Recently
shipped" above), and what's left of each is filed individually under
`docs/backlog/protocols/` at medium priority — the effective top of what's
actually open.

1. **Confirm `--gpu` fixes the Asahi Linux `--tty` failure** — needs the
   user's own hardware, not the dev VM (no split GPU/display-controller
   topology there). Unblocks the
   [`[tty] gpu` config key](docs/backlog/tty/tty-gpu-config-key.md).
2. **Medium-priority protocol gaps**, mostly what's left of the DMS/Noctalia
   probes (see "Shell enablement" below for their recommended order):
   [screencopy's toplevel half](docs/backlog/protocols/screencopy-toplevel-capture.md)
   (the output half shipped — see "Recently shipped" above),
   plus one filed from PR #44's own review —
   [popup grab serial validation](docs/backlog/protocols/popup-grab-serial-validation.md)
   (its sibling, [a window-focus change doesn't dismiss an active popup
   grab](docs/backlog/resolved/popup-grab-focus-divergence-done.md), shipped
   as an IPC `popup_grab` field in PR #55 — see "Recently shipped" above).
3. **[IPC connection cap and the half-closed-connection
   leak](docs/backlog/resolved/ipc-connection-cap-resolved.md)** — RESOLVED
   2026-09-16: 64 concurrent connections, refused with a reason past that,
   a write-stall deadline that drops a peer which has stopped reading (the
   leak a cap alone could not free), and — found by the review of that — a
   cap on how long a `wait-idle` may park a connection, without which 64
   parked waiters from a since-crashed agent wedged the whole control
   channel. The third concern that entry bundled — [capture and encode on
   the event-loop
   thread](docs/backlog/ipc/screenshot-encode-on-event-loop.md) — is re-filed
   and still open (medium priority), as are two adjacent things found while
   reviewing it: [the accept loop and
   `EMFILE`](docs/backlog/ipc/accept-loop-swallows-emfile.md) and [what a
   shared connection table costs an innocent
   client](docs/backlog/ipc/connection-cap-denies-the-same-user.md).
4. Small, unblocked low-priority fixes: [`msg key` modifier
   resolution](docs/backlog/input/msg-key-modifier-resolution.md),
   [`[binds]` capital
   letters](docs/backlog/config/binds-capital-letter.md),
   [`--width/--height`
   bounds](docs/backlog/core/width-height-unbounded.md).
5. [Rename `flexwm` → `flex`, split out
   `flexctl`](docs/backlog/meta/rename-flex-family.md) — decided, explicitly
   scheduled **last** in the burn-down, per the entry's own frontmatter.

## Shell enablement (DMS / Noctalia probes, 2026-09-14)

Two Quickshell shells were probed end to end
([DMS gaps](docs/backlog/protocols/dms-enablement-gaps.md),
[Noctalia results](docs/backlog/protocols/noctalia-probe.md)) after
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
   surface (measured). Follow-up filed:
   [grab serial validation](docs/backlog/protocols/popup-grab-serial-validation.md).
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
   to prefer). Reconfiguration is deliberately refused and re-filed as
   [its own item](docs/backlog/protocols/output-management-reconfiguration.md),
   gated on multi-output support: nothing an `apply` could ask for exists yet.
5. [`screencopy / image-capture`](docs/backlog/resolved/screencopy-capture-done.md)
   — HALF-RESOLVED 2026-09-16 (PR #52): `ext-image-copy-capture-v1` with
   `ext-image-capture-source-v1` for **output** capture, which is the
   workspace-overview preview half. The per-window thumbnail half needs a
   toplevel capture source (a second render target per session) and is
   [its own item](docs/backlog/protocols/screencopy-toplevel-capture.md).
   IPC screenshots stay regardless.
6. DMS unlock-path re-probe — done 2026-09-14 (see the re-probe note
   in the DMS gaps entry): lock → auth → unlock teardown survives on
   current `main`, and so does a spotlight open/Escape-dismiss cycle.
   Both shells' destroy kills are now proven fixed, not inferred.

Small follow-ups already filed alongside:
[`gamma-control`](docs/backlog/protocols/gamma-control-followups.md),
[`layer-destroy`](docs/backlog/protocols/layer-destroy-review-followup.md).

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
