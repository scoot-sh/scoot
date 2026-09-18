---
title: "The `ext_session_lock_manager_v1` global is offered to every client — CLOSED (accepted tradeoff, kept as a landing spot)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The `ext_session_lock_manager_v1` global is offered to every client — CLOSED (accepted tradeoff, kept as a landing spot).

## The entry as filed

`docs/backlog/protocols/session-lock-global-restriction.md` (17 lines, low
priority, blocked on `wp_security_context_v1` support): any client can bind
the lock manager and lock the session — a malicious client can lock the
screen at will or hold it locked. The protocol explicitly allows restricting
it ("the compositor may choose to restrict this protocol to a special
client"), Smithay's helper takes a client filter flexwm passes `|_| true`
to, and the entry concluded there is nothing to filter *on* — so it belongs
with the security-context protocol, not as a bespoke allow-list.

## Verify-first findings (2026-09-18, no code)

Re-derived against the pinned sources before writing anything, taking the
entry's framing (including its `blocked:` line) as a hypothesis, not a
verdict.

**1. The protocol defines no privilege, only the word.** The XML in tree
(`wayland-protocols` 0.32.13,
`protocols/staging/ext-session-lock/ext-session-lock-v1.xml`) was read, not
assumed: "This protocol allows for a privileged Wayland client to lock the
session" and "The compositor may choose to restrict this protocol to a
special client launched by the compositor itself or expose it to all
privileged clients, this is compositor policy." That is the entire
authorization model — "privileged" is never defined, no mechanism is named,
and nothing in the interface distinguishes a locker from any other client.
Restriction is policy with nothing to key on.

**2. The pinned Smithay filter is hide-from-registry, nothing more.** At
`0ff00983`, `SessionLockManagerState::new(display, filter)` stores the
filter in `SessionLockManagerGlobalData` and applies it only as `can_view`
(`src/wayland/session_lock/mod.rs`): `bind` ignores the client entirely,
and the `Lock` request handler takes no client predicate — once a client
sees the global it can lock. So the only restriction primitive is
registry-hiding, all-or-nothing per whatever predicate the compositor can
write over `&Client`.

**3. There is no true predicate — including after security-context.** The
entry's `blocked:` line does not survive contact with the pinned source:

- `Client::get_credentials` (pid/uid/gid) exists but carries upstream's own
  warning against security use ("it is possible for programs to spoof this
  kind of information", `wayland-server` 0.31.14 `src/client.rs`). And it
  would not distinguish even taken at face value: both sockets are
  owner-only (the IPC listener already enforces same-user; the Wayland
  socket lives in the same runtime dir), so every local client shares the
  compositor's uid, and pid is unstable by this codebase's own record
  (`bind_budget.rs`: zero in a PID namespace, reused after exit).
- `wp_security_context_v1` exists in Smithay at the pinned rev
  (`src/wayland/security_context/`), but flexwm does not advertise it — and
  advertising it would not unblock this ticket. It marks *sandboxed*
  connections (the manager global "must exclude clients created through a
  security context"), and its listener source yields bare `UnixStream`s with
  no built-in is-sandboxed query for a `can_view` filter to call: the
  compositor would hand-tag sandboxed connections itself, and every
  ordinary client — DMS, Noctalia, swaylock, and any attacker running as
  the user — would remain one undifferentiated class. Security-context
  scopes what sandboxes may do; it creates no "privileged" class for a
  locker allow-list to key on.
- There is no bind-time app identity anywhere else either: `app_id` is
  client-asserted, per-surface, and post-bind.

**4. Every candidate restriction fails concretely, and its failure lands on
the legitimate locker.** Refusal here has only two shapes, both bad for a
false positive: a protocol error kills the *connection* (the P0 class from
the post-destroy teardown), and a `can_view` hide silently denies the lock
screen altogether.

- *First binder wins:* a race the attacker can win on purpose, and a lock
  the legitimate locker can never take afterwards — including the takeover
  recovery (run a locker again after the red screen) that the abandoned-lock
  suite pins and both shell re-probes field-proved.
- *Compositor-spawned-only (pid tracking):* both probed shells connect as
  ordinary user-launched session processes (`nix shell ... -c dms run`,
  not a compositor child), pids are unstable per finding 3, and anything
  spawned breaks across restarts.
- *Allow-list (config knob, app-id match):* nothing true to key on per
  finding 3, so the knob would be bespoke theatre with a false-positive
  mode that kills or silently locks out DMS/Noctalia.
- *Bind budget (PR #74 interplay, per the ticket's watch item):* the
  session-lock manager is not one of the four capped globals, and capping
  binds could not gate `lock` anyway — one bind suffices for the attack,
  so a count bounds objects, not the nuisance.

**5. The threat model already concedes the boundary, and the demand side is
ordinary clients.** README's trust note states it: a same-uid process is
inside the boundary and always was — anything reaching the Wayland socket
can already read the user's files, so a screen-lock nuisance from inside
that boundary is not a new capability. The lock keeps *someone at the
keyboard* out, and that guarantee is unaffected by who may bind. On the
other side, the DMS and Noctalia lock flows are ordinary-client flows,
each field-proven through full lock → PAM auth → unlock cycles in the
September re-probes — and no available primitive distinguishes them from
an attacker.

## Resolution (closed without code, 2026-09-18)

The open global stands, as an accepted tradeoff rather than a deferred
defect — mirroring
[`output-management-reconfiguration-done.md`](./output-management-reconfiguration-done.md)
and
[`connection-cap-denies-the-same-user-done.md`](./connection-cap-denies-the-same-user-done.md).
Not "blocked on security-context" any more: finding 3 shows that protocol
alone unblocks nothing, so the ticket's `blocked:` line was wrong and is
superseded by this record rather than carried forward.

Revisit if any of these fires, in which case reopen from this record rather
than re-deriving it:

- a standard locker-privilege signal lands that both shells actually
  implement (a `can_view` predicate needs something true to key on, not
  just a new global);
- flexwm moves to spawning the locker itself as session design (the
  protocol's "special client launched by the compositor itself" half —
  a product decision, not a filter);
- a demonstrated malicious-lock incident changes the nuisance calculus
  (to date: zero attempts in any probe record; the only lockers on the
  wire are the real ones).

No fail-first tests: there is no behavior change to pin. What must keep
working — the ordinary-client lock path and the second-client takeover —
is already pinned by the harness suite both halves would break:
`session_lock/tests/lifecycle.rs` (lock/unlock through a real client),
`abandoned.rs` (a new client takes over an abandoned lock), and
`teardown.rs` (both teardown orders, both by-design kills).
