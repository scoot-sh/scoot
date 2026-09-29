---
title: "`ext-workspace-v1`: set the `urgent` state bit on a workspace holding a window that wants attention"
status: "open"
area: "protocols"
priority: "low"
blocked: "needs a policy decision first: what makes a window urgent (see below). Today an xdg-activation request always focuses the window."
---

# `ext-workspace-v1`: the `urgent` state bit

Filed 2026-09-29 (scootbar planning). Serves **daily-drive**: a bar can mark a
workspace whose window is asking for attention, using the standard protocol
rather than a scoot-specific event (`CLAUDE.md`: implement the standard where
one exists).

## The gap

`workspace_state` in `compositor/ext_workspace.rs` only ever sets `active`.
Its doc comment says `urgent` is never set because "there is no
`xdg_activation` support" — that reason went stale when `xdg-activation-v1`
landed (`compositor/activation.rs`); refresh the comment whichever way this
ticket goes.

## The real question: what is urgent?

`request_activation` (`activation.rs`) validates the token and then
**focuses the window immediately**, scrolling it into view. It never marks a
window as merely wanting attention. So setting `urgent` is not a wiring job;
it needs a policy, and the choice changes behavior users already have:

1. **Keep focusing; urgent never fires.** Status quo. Close this ticket as a
   deliberate refusal.
2. **Focus only when the token proves user interaction** (it already carries
   a real, recent serial, `activation-serial-validation-done.md`), and
   otherwise mark the window urgent without moving focus. Cost: a client
   that activated itself with a stale token stops getting focus, which
   is the point, but needs checking against the shells and launchers in
   use (DMS, Noctalia, quickshell) before it lands. (Check the
   `xdg-activation-v1` text for what it says a compositor should do with
   a token it cannot tie to a seat, rather than recalling it.)
3. **Urgent from another source** (a bell, `_NET_WM_STATE_DEMANDS_ATTENTION`
   for XWayland windows) as an addition to either of the above.

Recommend deciding 1 vs 2 with the shells' behavior measured, not assumed.

## If it proceeds

- The core needs a per-window urgent flag that clears on focus, and a
  workspace is urgent while any of its windows is and it is not active.
- `workspace_state` takes it; it rides the existing batched `done`, so no
  new wire shape.
- `msg windows` should report it too, so an agent sees what a bar sees.
- A workspace adopted from an unplugged monitor keeps its windows' flags.

## Not in this ticket

`hidden`, which stays unset for the reason `workspace_state` documents.
