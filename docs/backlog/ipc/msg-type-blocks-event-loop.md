---
title: "A large `flexwm msg type` blocks the whole event loop for its whole duration (LOW, pre-existing)."
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# A large `flexwm msg type` blocks the whole event loop for its whole duration (LOW, pre-existing).

A large `flexwm msg type` blocks the whole event loop for its whole
duration (LOW, pre-existing). `ipc.rs` runs `Request::Type` to
completion synchronously on the sole event-loop thread, so a request at
the 1 MiB inbound cap (item 9's `ipc/line.rs` limit, which is what bounds
how big this can get) stalls wayland dispatch, input and rendering until
every character has been sent. Pre-existing — `type_text` has always been
synchronous, and `git blame` puts it well before the shifted-character
work — but PR #23's own benchmark numbers put a figure on it and roughly
doubled the worst case: ~2.3 s per MiB before, ~2.4 s if the text is all
lowercase, ~3.9 s if it is all shifted (extrapolated from the measured
medians of 112 ms and 188 ms per 50,000 characters, release build,
`--headless`), because a shifted character correctly costs four key
events instead of two. Nothing an agent does deliberately gets near this
— a shell command line is a few hundred characters, i.e. under a
millisecond — so this is a hostile-input/accident bound, not an everyday
cost. Fixing it properly means chunking the request across event-loop
iterations (send N characters, yield, resume), which needs per-connection
progress state of the kind item 10 deliberately avoided; a much cheaper
partial answer is a separate, smaller cap on `Request::Type`'s text
specifically, refused rather than delayed, the same shape the screenshot
rate limit took.
