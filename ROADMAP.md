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
   [foreign-toplevel management](docs/backlog/protocols/foreign-toplevel-management.md),
   [output-management](docs/backlog/protocols/output-management.md),
   [screencopy/image-capture](docs/backlog/protocols/screencopy-capture.md),
   plus two filed from PR #44's own review —
   [popup grab serial validation](docs/backlog/protocols/popup-grab-serial-validation.md)
   and [a window-focus change doesn't dismiss an active popup
   grab](docs/backlog/protocols/popup-grab-survives-window-focus-change.md).
3. **[IPC screenshot capture: synchronous, no connection
   cap](docs/backlog/ipc/screenshot-sync-no-rate-limit.md)** (medium
   priority).
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
3. [`foreign-toplevel`](docs/backlog/protocols/foreign-toplevel-management.md)
   — window lists (check for an `ext-` successor first).
4. [`output-management`](docs/backlog/protocols/output-management.md)
   — shell display/settings pages.
5. [`screencopy / image-capture`](docs/backlog/protocols/screencopy-capture.md)
   — thumbnails/overview previews (filed 2026-09-14; IPC screenshots
   stay regardless).
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
