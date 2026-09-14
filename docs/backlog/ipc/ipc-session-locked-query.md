---
title: "No IPC way to ask whether the session is locked"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# No IPC way to ask whether the session is locked

No IPC way to ask whether the session is locked (item 18). An agent
driving flexwm can tell indirectly — `flexwm msg action ...` answers
`refused: the session is locked ...`, and a screenshot shows the lock
screen — but there is no request that says so directly. A new `Response`
*variant* does break older clients' decoding and so needs a
`PROTOCOL_VERSION` bump; a defaulted `#[serde(default)]` field does not
(output scaling added `OutputSnapshot.scale` that way, with no bump). So
the cheapest shape here is a defaulted `locked: bool` on the existing
`Response::Ok` (or on the output snapshot), not a new variant — and it
lands cleanly alongside the `usable` rect and the "focus workspace N"
action.
