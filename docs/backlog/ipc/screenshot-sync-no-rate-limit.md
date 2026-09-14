---
title: "Screenshot capture runs synchronously on the sole event-loop thread with no rate limit (MEDIUM)"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
---

# Screenshot capture runs synchronously on the sole event-loop thread with no rate limit (MEDIUM)

Screenshot capture runs synchronously on the sole event-loop thread with
no rate limit (MEDIUM) — the rate limit is DONE as item 9 (one
capture per connection per 16ms frame, refused rather than delayed). Still
open, and deliberately out of scope there: the IPC accept loop has no cap
on concurrent connections, so the per-connection limit is bypassable by
reconnecting for every capture, and nothing bounds how many connections one
client can hold open. Also still true, and unaffected by rate limiting: the
capture itself is synchronous on the event-loop thread, so each one stalls
wayland dispatch and input for its duration (~12ms at 1600x1000 in a
release build, measured in item 9). Moving the encode off-thread is a much
larger change than the limit was. One more lifecycle case belongs here,
found by `flexwm-reviewer` while reviewing item 10: a client that
half-closes (`shutdown(SHUT_WR)`) and then never reads pins its connection
slot and two fds for good. A half-close raises `EPOLLIN`/`EPOLLRDHUP`, not
`EPOLLHUP`, and a connection with a queue is registered for writability
only, so nothing wakes it again. Deliberately not fixed in item 10:
registering for reads there would spin the loop at full speed on an
end-of-stream that can never be acted on (the queue cannot drain), which is
worse. Strictly better than the pre-item-10 behavior, where that same case
froze the whole compositor — but still a live resource leak a connection
cap would need to account for.
