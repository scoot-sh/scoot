---
title: "scootlaunch: two simultaneous opens can orphan a panel; socket mode implicit"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scootlaunch: two simultaneous opens can orphan a panel; socket mode implicit

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #674 (F4, F5). Make connect-or-bind atomic (lock file or O_EXCL), so two opens in the same instant yield one panel. Set the socket to 0600 explicitly instead of relying on umask, with a test.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if the change is user-visible; otherwise say so in the PR.
