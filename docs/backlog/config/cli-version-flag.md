---
title: "No `scoot --version`: a build can only be identified by starting it"
status: "open"
area: "config"
priority: "low"
blocked: null
---

# No `scoot --version`: a build can only be identified by starting it

Filed as gh issue #176 (read it — short). `scoot --version` answers
`unknown argument`; the only version signal is `scoot msg version`,
which needs a running compositor. From a packaging/triage point of view
an installed binary cannot identify itself.

Fix shape (from the issue): `--version` printing the same
`env!("CARGO_PKG_VERSION")` the IPC reply uses (agree by construction),
plus the IPC protocol number alongside (`scoot 0.1.0 (ipc protocol 3)`)
so a `scootctl` user can check compatibility against a remote
compositor before connecting. That means: `scoot --version`,
`scootctl --version` (or `scootctl version`? decide — the client grammar
post-split takes bare request words, so pick the spelling deliberately
and record it), `--help` lines for both, tests pinning the exact output
(including the protocol number tracking `PROTOCOL_VERSION` — a test that
fails when the constant moves without the string is the pin, or derive
it in code so it cannot drift).

Scope: flag + output + tests + `--help`/docs lines. No wire change.
