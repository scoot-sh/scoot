---
title: "IPC socket has no explicit permissions or peer-credential check (LOW/MEDIUM) \u2014 DONE as item 9."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# IPC socket has no explicit permissions or peer-credential check (LOW/MEDIUM) — DONE as item 9.

~~IPC socket has no explicit permissions or peer-credential check
(LOW/MEDIUM)~~ — DONE as item 9. Both halves: `0600` on the socket file
and a same-uid `SO_PEERCRED` check at accept time, proven independent of
each other on hardware. One correction to this entry's diagnosis, found
while fixing it: under the ordinary `umask 022` the socket came out
`srwxr-xr-x`, which other users cannot connect to anyway (connecting needs
write permission) — so "silently degrades to any-local-user access" was
true for a lax umask (demonstrated with `umask 000`), not for a
`FLEXWM_SOCKET` override alone.
