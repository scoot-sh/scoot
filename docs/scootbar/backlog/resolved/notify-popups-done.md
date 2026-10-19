---
title: "scootnotify: cards, the stack, lock and idle rules, outputs, the look layer"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootnotify: popups

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#behavior-on-screen). Serves **daily-drive**.
`look-format` is the live-looks `scoot-look` crate (#578, merged).

## What to do

- **The card and stack** on one on-demand `overlay` layer surface, with the input region set to
  the cards' union.
- **Pointer actions**, with `ActivationToken` emitted before `ActionInvoked`.
- **Expiry** started on the first frame callback that presents a card, a scoot-specific
  guarantee the docs state as such. Cards already on screen keep expiring through a manual
  lock, and the docs say that too.
- **Pauses**: on hover, and on input idle through `ext-idle-notify` v2.
- **Output**: a null output (the compositor's choice; `layer-shell-default-output` makes that
  the user's output) or a named one; hotplug; zero outputs.
- **Config** `notify.toml` (style, placement, timeouts) and **the look layer** under it, per the
  live-looks contract: `<state>/look/notify.toml`, the allowlist, the value grammar, a dangling
  link meaning no layer. The re-theme-in-place path is built here; the `msg reload` verb is in
  `notify-control`.

## Acceptance

Headless-harness tests:

- pixels per card state (test font);
- a click invokes, with the token first;
- a card arriving while locked is not drawn, and its clock does not start until after unlock;
- unplugging an output moves the stack;
- the surface and buffers are gone when empty (fds and maps checked).

Also: zero allocation per repaint, and 0 idle wakeups with no cards.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/index.md and a new site/src/content/docs/scootnotify/configure.md` with popup placement, timeouts, urgency, the look layer and the conformance table row changes. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The ratchet rows from the design's budgets table against mako.

## Not in this ticket

The control socket, DND, rules, history, the bar.

## Resolution

Shipped in PR #683 (`feat/notify-popups` branch): the card stack on
one on-demand `overlay` layer surface, pointer actions with the token
first, frame-callback presentation, hover and input-idle pauses, null
or named outputs with hotplug, `notify.toml` placement/timeout keys,
and the `notify.toml` look layer with the re-theme path (`msg
reload` itself is `notify-control`).

What landed:

- `view/layout` (newest on top, `max_visible` + "+N more" pill, wrap
  with ellipsis, the cards'-union input region) and `view/render`
  (urgency ring, fills, text, icons, progress, buttons, pill), with a
  mutation-proved zero-allocation repaint pin under the binary's own
  counting allocator.
- `popup`: the on-demand surface (created on the first card,
  destroyed with unmapped buffers on the last), pointer hits (body,
  button, right, middle), `ActivationToken` minted from the click's
  serial and emitted before `ActionInvoked`, clocks starting on the
  frame callback (arrivals while locked or output-less wait stopped),
  hover pause per card and `ext-idle-notify` v2 pause for all, named
  output with compositor-choice fallback, and hide-on-close so the
  stack re-shows.
- `notify.toml` gains placement and style keys (all optional);
  `scoot-look render_notify` plus the apply / build / cache / sync
  paths (`RENDER_VERSION` 12): every render carries `notify.toml`
  with the thirteen shell-app roles, the UI font at its popup size,
  the popups opacity and the look radius.
- Site `scootnotify/index.md` rewritten for popups, new
  `scootnotify/configure.md` (task-first, type/default/example per
  option, symptom boxes), sidebar entry; snippet gate green.
- CI: `scootui` and `scoot-look` fan out to the `scootnotify` job,
  which builds scoot first and runs the headless popup suite with
  `SCOOTNOTIFY_REQUIRE_SCOOT/SWAY/DBUS_DAEMON`.

Evidence (dev VM, aarch64): the headless suite (pixels per state,
token-first click, locked arrival held, hotplug move, empty
surface/buffers, no-wakeup idle) green on both runners; the three
mutation proofs red-then-green (repaint allocs 3 vs 0, token order
`["invoke","token","closed"]`, locked expiry unseen); ratchet rows
in the PR body against mako. Bugs found by testing, fixed: pixels
before the initial configure ack (compositor kills the client),
surface pinned to the first card's size, a Release-driven redraw
loop, a close that never re-showed.
