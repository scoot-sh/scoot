---
title: "IPC socket path is unlink-then-bind (LOW, non-default config only) \u2014 DONE as item 9."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# IPC socket path is unlink-then-bind (LOW, non-default config only) — DONE as item 9.

~~IPC socket path is unlink-then-bind (LOW, non-default config only)~~ —
DONE as item 9. The original framing here was wrong (`bind(2)` does not
follow a trailing symlink, verified — `EADDRINUSE`), so the real exposures
were the chmod-after-publish window and losing the name to a racing
process. The fix that shipped is not a simple bind-to-temp-name-then-
rename, though: two review rounds found that shape itself introduced two
more symlink/length bugs of its own (see item 9's own write-up for the
full history). The final design claims an unpredictable
(`getrandom`-sourced), short staging name via `mkdir` itself — never an
unlink or a name a pre-planted symlink could sit at — then does every
subsequent operation (chmod, bind, the rename's source side) through an
`O_DIRECTORY | O_NOFOLLOW` fd via `/proc/self/fd/<n>/...`, never
re-resolving the staging path by name again. Only the final rename's
*destination* (the published path itself) is still name-resolved, which
is inherent to what a published path is.
