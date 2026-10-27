---
title: "scootlaunch: two simultaneous opens can orphan a panel; socket mode implicit"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# scootlaunch: two simultaneous opens can orphan a panel; socket mode implicit

Filed 2026-10-19. Serves **daily-drive** (a double-pressed bind must not strand an unreachable panel; the toggle socket must not be connectable by another user).

## Resolution

Landed 2026-10-27 in `crates/scootlaunch/src/socket.rs`: the toggle is now claimed under an exclusive nonblocking `flock` on `scootlaunch-<display>.lock` (held for the panel's life), so two simultaneous opens serialise — the winner binds and listens, the loser yields without opening, leaving one panel instead of orphaning the first (its file unlinked from under its listener). The socket is published `0600` with no umask-dependent window: built inside a freshly-created `0700` staging directory, chmodded before the atomic rename onto the published name (as `scoot/src/compositor/ipc/listener.rs` does), never chmod-after-bind. A regular file at the socket path is refused, not removed; a stale socket is taken over.

Evidence: `socket::tests::two_simultaneous_opens_yield_one_panel` (barrier, no sleeps: before fix 2 Listens, after 1 Listen + 1 Signalled) and `socket::tests::socket_is_owner_only` (before fix mode `0755` under `022`, after `0600`) both failed before and pass after; `nextest -p scootlaunch` 315 passed, `cargo test -p scootlaunch` green, `clippy -D warnings` and `fmt --check` clean. The shared `scootui::control::Claim` bind-then-chmod window (bar/notify) and its `scootbg` duplicate are out of scope here and filed as `control-socket-mode-window`.

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #674 (F4, F5). Make connect-or-bind atomic (lock file or O_EXCL), so two opens in the same instant yield one panel. Set the socket to 0600 explicitly instead of relying on umask, with a test.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if the change is user-visible; otherwise say so in the PR.
