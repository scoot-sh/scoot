---
title: "`ext-session-lock-v1` protocol support \u2014 DONE as item 18"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `ext-session-lock-v1` protocol support — DONE as item 18

~~`ext-session-lock-v1` protocol support~~ — DONE as item 18, PR #25.
The entry as written asked whether the pinned Smithay rev had a helper: it
does (`wayland::session_lock`), so the protocol plumbing came from there
and flexwm wrote the policy. See item 18 for what shipped, what was
deliberately deferred, and the crash-recovery decision.
