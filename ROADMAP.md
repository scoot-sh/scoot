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

## What's next

The backlog is the source of truth for what to pick up; this is the current
read of it, not a commitment:

1. **Confirm `--gpu` fixes the Asahi Linux `--tty` failure** — needs the
   user's own hardware, not the dev VM (no split GPU/display-controller
   topology there). Unblocks the
   [`[tty] gpu` config key](docs/backlog/tty/tty-gpu-config-key.md).
2. **[`ext-idle-notify-v1` + `idle-inhibit-unstable-v1`](docs/backlog/resolved/ext-idle-notify-resolved.md)** —
   RESOLVED 2026-09-15: the automatic trigger session-lock had no other
   way to get. A `swayidle`-style daemon can now idle, resume and
   re-idle the seat (field-proven live); inhibitors hold it awake.
3. **The `PROTOCOL_VERSION` bundle** — `OutputSnapshot.usable`
   ([#1](docs/backlog/ipc/msg-outputs-usable-rect.md)), a focus-workspace-N
   action, and a locked-session query
   ([#2](docs/backlog/ipc/ipc-session-locked-query.md)) all wait on the same
   wire bump; land them together.
4. Small, unblocked fixes: [`msg key` modifier
   resolution](docs/backlog/input/msg-key-modifier-resolution.md),
   [`[binds]` capital
   letters](docs/backlog/config/binds-capital-letter.md),
   [`--width/--height`
   bounds](docs/backlog/core/width-height-unbounded.md).

## Shell enablement (DMS / Noctalia probes, 2026-09-14)

Two Quickshell shells were probed end to end
([DMS gaps](docs/backlog/protocols/dms-enablement-gaps.md),
[Noctalia results](docs/backlog/protocols/noctalia-probe.md)) after
landing gamma-control, both data-controls and primary selection
(PR #32) plus two destroy-teardown kill fixes (PRs #34, #36). Both
shells render fully; Noctalia is the better target (generic
`ext-workspace-v1` backend, richer IPC). Remaining gaps, in the
probes' recommended order:

1. [Popup input](docs/backlog/protocols/xdg-popup-input.md)
   — popups map and draw since the initial-configure fix (resolved:
   [`xdg-popup-initial-configure`](docs/backlog/resolved/xdg-popup-initial-configure-resolved.md)),
   and pointer clicks already land on them via the window hit-test, but
   keyboard focus never moves onto one, grabs are a no-op (so nothing
   dismisses a menu either), and layer-surface-parented popups (a bar's
   own menus/tooltips) are still untracked.
2. Item 2 above (idle) — RESOLVED 2026-09-15: auto-lock's trigger
   exists and is field-proven with real swayidle; what remains is the
   user's own daemon config, not compositor work.
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
