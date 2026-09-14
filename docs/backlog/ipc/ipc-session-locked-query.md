---
title: "No IPC way to ask whether the session is locked"
status: "open"
area: "ipc"
priority: "medium"
blocked: "bundle with PROTOCOL_VERSION bump"
---

# No IPC way to ask whether the session is locked

No IPC way to ask whether the session is locked (item 18). An agent
driving flexwm can tell indirectly — `flexwm msg action ...` answers
`refused: the session is locked ...`, and a screenshot shows the lock
screen — but there is no request that says so directly. Deferred because
a new `Response` variant (or a field on an existing one) breaks older
clients' decoding, so it needs a `PROTOCOL_VERSION` bump: bundle it with
the "focus workspace N" action and the `OutputSnapshot` `usable` rect,
which are waiting on the same bump.
