---
title: "IPC control socket has no line-length cap (MEDIUM) \u2014 DONE as item 9"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# IPC control socket has no line-length cap (MEDIUM) — DONE as item 9

~~IPC control socket has no line-length cap (MEDIUM)~~ — DONE as item 9,
for the compositor's inbound request path only (`ipc/line.rs`, 1 MiB).
`flexwm-ipc`'s `read_message`/`read_message_buffered` is deliberately left
uncapped: the same codec reads `Response::Screenshot`, which is legitimately
several MB of base64 PNG, so a cap there would break real screenshots. A
future `Client`-side cap would have to be per-message-type, not global.
