---
title: "XWayland: a watching X client can race a launched app to its startup id"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: bind startup-id redemption to the spawned process

Filed 2026-09-25 from the PR #244 (XWayland Phases 2+3) completion review.
Serves computer use and daily use alike: the focus gate is a security
property, and this is its one known window.

## The gap

The X focus gate (`compositor/xwayland/focus.rs`) lets a mapping X window
take focus when its `_NET_STARTUP_ID` names a live activation token --
`State::spawn` hands every child its token as `DESKTOP_STARTUP_ID` while
XWayland is live. But a startup id is a *property on the launched app's
window*, readable by every X client. A background X client that watches the
root for `CreateNotify` can read a freshly launched app's `_NET_STARTUP_ID`,
copy it onto a window of its own, and map first: it redeems the token and
takes focus from a Wayland window once. The real app arrives second and is
refused (the token is single-use). The window is as long as the token is
live: up to 30 s after the launch (`TOKEN_LIFETIME`), until the app's own
window spends it.

The redemption does not check the redeeming window's process. The existing
test (`a_spawn_tokens_startup_id_lets_an_x_window_take_focus_once`) shows it:
the redeeming X client is the test process, not a spawned child.

(A same-uid process can also read any child's token from
`/proc/<pid>/environ`. That is the project's documented same-uid trust
boundary and applies to every activation token, X or Wayland; out of scope
here.)

## Proposed tightening

When the token carries `SpawnedPid(p)` (every token scoot mints for a spawn
while XWayland is live), accept a startup-id redemption only if the window's
X-Resource client pid is `p` or a descendant of it -- a bounded walk up
`/proc/<pid>/stat` ppids (a few levels), so wrapper scripts (`sh -c`,
launcher shims, `flatpak run`) keep working. A token without `SpawnedPid` (a
Wayland launcher's, minted from a real click) keeps today's rule, or is
bound to nothing and refused for X -- decide and document.

Cost: one `/proc` walk per startup-id redemption (a map, not a hot path).
Test: a second X client (another process -- `xeyes`, or a helper binary)
copying the id and mapping first must be refused, and the real child still
focused.

## What done looks like

The race test fails first against today's gate and passes after; the
`focus.rs` and `protocols.md` "one known window" notes are removed; full
gate green.
