---
title: "scootnotify: cards, the stack, lock and idle rules, outputs, the look layer"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-server, scootui-surfaces-on-demand, look-format"
milestone: "M7"
---

# scootnotify: popups

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#behavior-on-screen). Serves **daily-drive**.
`look-format` is #578's `scoot-look` crate; it exists once #578 merges (#578 first).

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

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The ratchet rows from the design's budgets table against mako.

## Not in this ticket

The control socket, DND, rules, history, the bar.
