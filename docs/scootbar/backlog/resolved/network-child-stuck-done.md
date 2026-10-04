---
title: "Network: a connect or menu command that never exits blocks the next one"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Network: a connect or menu command that never exits blocks the next one

Filed 2026-10-03, from the independent review of PR #403. Serves
**daily-drive**: a stuck connect leaves the WiFi list unable to connect
again until the bar restarts.

## The gap

`connect N` refuses a second connect while the first
`connect-command` child is still running ("a connect is already
running"), and the same holds for the dmenu picker's `menu-command`.
Neither has a timeout or a way to end the child: a command that hangs
(an `nmcli` waiting on a secret agent that never answers, a picker
left open on another workspace) blocks every later connect, and a
module removed by a reload leaves the child running, unreaped, because
nothing kills it on drop. Both paths behave the same way, so this
is not a #403 regression; the review flagged it as worth deciding.

## What to do

Decide, with the user-facing behavior spelled out in `cli.md`:
either a new `connect` replaces a running one (end the old child, then
start the new), or a time bound after which the child is ended and the
module says so on stderr; and end both children when the module is
dropped. Tests: a `sleep infinity` command, then a second connect; a
reload while a child runs; a child that ignores SIGTERM.

## Resolution (2026-10-04, PR #428)

Decided for replacement over a timeout: a second `connect` or `menu`
while one runs ends the running child and starts the new one — no
timeout to tune, and the latest intent always wins. Ending is `SIGTERM`
to the child's own process group (both spawns now lead one via
`process_group(0)`), then `SIGKILL` after a bounded 100 ms grace, reaped
either way; dropping the module ends both children the same way. Only a
command that will start ends the old one. Supporting change the ticket
needed: the kept scan list now resets on the re-dump's first page (or
its terminator when empty) instead of at queue time, so a replacement
menu is fed what the first one was. Behavior spelled out in
`docs/scootbar/cli.md` Network, as the ticket asked.

Evidence: four tests, each failing before the fix and passing after on
the Asahi M2 (`cargo test -p scootbar --bin scootbar modules::network`:
77 passed) — `a_second_connect_replaces_the_running_one`,
`a_second_menu_replaces_the_running_one`,
`dropping_the_module_ends_both_running_children`,
`a_child_that_ignores_sigterm_is_killed`. Clippy clean across 37 feature
sets; the full matrix rode CI on the PR head (the shared box's disk hit
100% mid-run under another lane's closure build).

## Not in this ticket

The PR #403 review's other note: `popup/paint.rs`'s ellipsis `cut`
restates `modules/ellipsis.rs`'s for a bigger buffer. One generic over
the buffer size would remove the copy and a latent bound both share
(writing the 3-byte ellipsis needs `len <= N - 3`; the review found it
unreachable with real labels). Small; take it with whichever change
next touches either file.
