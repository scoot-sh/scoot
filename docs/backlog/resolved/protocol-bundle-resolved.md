---
title: "IPC bundle: `OutputSnapshot.usable`, `focus-workspace-index`, ambient `locked` on `Ok` — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# IPC bundle: `OutputSnapshot.usable`, `focus-workspace-index`, ambient `locked` on `Ok` — RESOLVED.

## Resolution (2026-09-15)

All three landed together, with **no `PROTOCOL_VERSION` bump** -- the
bundle's premise ("all wait on the same wire bump") did not survive the
entries' own analysis, and the analysis won:

- **`OutputSnapshot.usable: Rect`** (`flexwm-ipc`, populated in
  `State::output_snapshots` from `World::usable_area`). Defaulted like
  `scale`, but the default is a sentinel rather than a truthful value:
  all zeros, which no real output can have. An older server *did*
  reserve bars, so defaulting to "same as `rect`" would be a lie; the
  sentinel tells a new client to fall back to `rect`, exactly what it
  did before the field existed. Old clients ignore the unknown field.
- **`Action::FocusWorkspaceIndex { index: usize }`** + `convert.rs` arm
  + `msg action focus-workspace-index N` spelling (0-based -- note the
  off-by-one against the bar-facing side: `ext-workspace-v1` `name`s
  workspaces 1-based for display while this index is 0-based, and
  neither side adjusts; out of range does nothing, same as the core
  action `ext-workspace-v1`'s `activate` already drives). A separate
  name rather than overloading `focus-workspace`, so `up`/`down` and `2`
  never parse ambiguously -- pinned by a cli test. New `Action`
  variants are wire-safe without a bump for a directional reason, and
  the direction matters: `Action` travels client→server, so an old
  client (which never sends the new variant) never trips on it. The
  reverse does *not* hold -- a new `Response` variant travels
  server→client and *would* break an old client with a hard decode
  error, which is exactly what forced the bump to 2 for
  `Response::Warning` (see `PROTOCOL_VERSION`'s doc). Related and
  accepted: since there is no bump, `msg version` (still protocol 2)
  cannot tell a pre-bundle server from a post-bundle one, so capability
  discovery is try-it-and-fall-back -- an agent sends
  `focus-workspace-index` and, on an error reply, steps with
  `focus-workspace up|down`. That was already true of every prior
  additive action spelling, so this follows standing practice rather
  than inventing it.
- **`Response::Ok { locked: bool }`**, filled at all seven `Ok` sites
  through one `State::ok()` constructor so no future site forgets it.
  Read asymmetrically: `true` is always truthful, `false` means
  unlocked *or* a predating server. The field strictly adds information
  (a `true` is new knowledge) without breaking a single existing
  client.

Coverage: wire tests pin both compat directions (old JSON without the
fields decodes to the defaults; a new reply decodes under a struct
lacking the fields, the way an old client would); a layer-shell test
asserts `rect` stays whole while `usable` narrows under a 30px bar; a
session-lock test asserts `Ok{locked:true}` for input served while
locked and `Ok{locked:false}` after unlock; cli + convert tests pin the
new spelling end to end. Three pre-existing tests matching `Response::Ok`
exactly were updated to the struct pattern.

Also folded in: `ipc-session-locked-query.md` (no direct locked query;
the ambient flag is the cheapest shape that answers it -- an agent
asking "did the unlock land" reads the next reply rather than polling).
There is still no *request* that asks locked-ness directly; that would
be a new `Response` variant, and the analysis above says even that
would not need a bump -- but nothing needs it while the flag rides
every reply.

Original entries, left as written:

## 1. `flexwm msg outputs` reports only an output's full rectangle

`flexwm msg outputs` reports only an output's full rectangle, so an
agent cannot see what a bar reserved (item 14 gave the core a `usable`
area but did not extend the IPC surface). Adding a `usable` rect to
`OutputSnapshot` is a one-field change to `flexwm-ipc`. It does **not**
need a `PROTOCOL_VERSION` bump: output scaling added the `scale` field the
same way — `#[serde(default)]`, so an old server's reply still decodes and
the tagged `Response` discriminant is unchanged. This entry previously
assumed a bump; the `scale` field disproved that premise. It is still
worth landing alongside an IPC action for "focus workspace N", which
item 15 deferred for the same reason: `flexwm_core::Action::FocusWorkspaceIndex`
already exists (it is what `ext-workspace-v1`'s `activate` drives), so all
that is missing is the wire half — a variant on `flexwm-ipc`'s own
`Action` mirror carrying the index, its `convert.rs` arm, its
`msg action` spelling in `cli.rs`, and its `README.md` row. Until then an
agent can only step workspaces one at a time (`focus-workspace up|down`)
while a bar speaking `ext-workspace-v1` can jump straight to one. Landing
them together means one review of the wire surface, not one forced bump.

## 2. No IPC way to ask whether the session is locked

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
