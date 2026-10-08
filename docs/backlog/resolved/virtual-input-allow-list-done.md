---
title: "Per-client allow-list for the virtual-input globals via security-context-v1"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Per-client allow-list for the virtual-input globals via security-context-v1

Filed 2026-10-05 from PR #448's review. Serves **daily-drive**: a user
running wayvnc wants only wayvnc able to drive the session, not every
same-uid process.

## The gap

`[virtual_input] enabled` advertises the virtual-pointer and
virtual-keyboard globals to every same-uid client: any process that can
bind them can type and click as the user
(`crates/scoot/src/compositor/virtual_input.rs`, Gating section;
`site/src/content/docs/scoot/protocols.md` trust note). An allow-list
naming privileged clients would be theatre today -- scoot has no
`security-context-v1` support, so it cannot tell wayvnc from any other
same-uid client -- which is why #448 ships the on/off switch as the whole
gate. The switch answers the network threat (wayvnc listens on TCP), not
the local-client one.

## What to do

Once scoot supports `security-context-v1`, gate the two virtual-input
globals on it: advertise (or accept binds from) only clients carrying a
security context the user allow-listed, keeping the current switch as the
default-deny underneath. Pin with tests: an unlisted same-uid client sees
no globals (or its bind fails) while a listed one drives the session; the
lock-gate and the destroy/disconnect release paths keep working for
listed clients.

## Not in this ticket

Implementing `security-context-v1` itself; changing the switch default;
per-window or per-seat scoping (see `native-remote-scene-streaming.md`
for the fuller remote story).

## Verify-first findings (2026-10-08, no code)

Re-derived against the pinned sources before writing anything, taking the
ticket's framing ("an allow-list needs security-context support first") as
a hypothesis, not a verdict. It does not survive contact with either the
record or the source.

**1. The precondition is out of scope and deliberately unbuilt.** The
ticket itself puts implementing `security-context-v1` outside it, and
that protocol was closed as a deliberate non-entry on 2026-09-18
(`docs/backlog/resolved/protocol-gaps-niche-done.md`, item 4): Smithay
carries it at the pinned rev, but advertising it honestly needs a
sandbox story scoot has none of, with no Flatpak/sandbox demand in
either probe record. There is no open ticket to build it, so "once scoot
supports `security-context-v1`" has nothing to wait on.

**2. `security-context-v1` would not unblock this ticket anyway.** At the
pinned fork rev (`scoot-sh/smithay` `fdf424d`, the rev
`crates/scoot/Cargo.toml` pins), `SecurityContextState::new` stores the
filter in `SecurityContextGlobalData` and applies it only as `can_view`
("It *must* exclude clients created through a security context for the
protocol to be correct and secure"), and the committed context yields a
`SecurityContextListenerSource` whose events are bare `UnixStream`s with
no built-in is-sandboxed query for a `can_view` filter to call: the
compositor hand-tags sandboxed connections itself
(`src/wayland/security_context/mod.rs`, `listener_source.rs`). It marks
*sandboxed* connections. wayvnc is not one: it connects as an ordinary
client over `WAYLAND_DISPLAY`
(`site/src/content/docs/scoot/remote-desktop.md` runs
`WAYLAND_DISPLAY=wayland-1 wayvnc`), the same undifferentiated class as
every same-uid process and any attacker running as the user. The
protocol scopes what sandboxes may do; it creates no "privileged" class
for a wayvnc allow-list to key on. This is the same finding that closed
the session-lock allow-list without code
(`docs/backlog/resolved/session-lock-global-restriction-done.md`,
finding 3).

**3. There is no other true predicate either.** `Client::get_credentials`
exists but carries upstream's own warning against security use ("it is
possible for programs to spoof this kind of information",
`wayland-server` 0.31.14 `src/client.rs`), and it would not distinguish
even taken at face value: the Wayland socket lives in the same runtime
dir as every local client, so all of them share the compositor's uid.
`app_id` is client-asserted, per-surface, and post-bind. The two
virtual-input managers take no predicate at all today (`can_view` is
`|_| true` in `crates/scoot/src/compositor/virtual_input.rs` for both
`VirtualPointerManagerGlobalData` and
`VirtualKeyboardManagerGlobalData`): the only restriction primitive is
registry-hiding, all-or-nothing per whatever predicate the compositor
can write over `&Client`, and there is nothing true to write it over.

**4. Every candidate allow-list fails concretely, and its failure lands
on wayvnc.** A config knob naming privileged clients keys on nothing
true per finding 3, so it is bespoke theatre with a false-positive mode:
a `can_view` hide silently denies wayvnc the globals (it falls back to
`--disable-input` view-only or refuses to start), while a bind-time
refusal is a protocol error that kills the *connection*. First-binder
wins is a race the attacker wins on purpose. Compositor-spawned-only
(pid tracking) breaks across restarts and on pid instability this
codebase already records (`bind_budget.rs`: zero in a PID namespace,
reused after exit) -- and neither probed VNC flow is a compositor child.

**5. The threat model already concedes the boundary, and the demand side
is an ordinary client.** The site trust note states it
(`site/src/content/docs/scoot/protocols.md`): a same-uid process is
inside the boundary and always was -- anything reaching the Wayland
socket can already read the user's files, so virtual-input abuse from
inside that boundary is not a new capability. The ticket itself says the
switch answers the network threat (wayvnc listens on TCP), not the
local-client one.

## Resolution (closed without code, 2026-10-08)

The on/off switch stands as the whole gate, as an accepted tradeoff
rather than a deferred defect -- mirroring
[`session-lock-global-restriction-done.md`](../resolved/session-lock-global-restriction-done.md).
Not "blocked on security-context" any more: finding 2 shows that
protocol alone unblocks nothing, so the ticket's `blocked:`-by-implication
was wrong and is superseded by this record rather than carried forward.

Revisit if any of these fires, in which case reopen from this record
rather than re-deriving it:

- a standard privileged-client signal lands that wayvnc actually
  implements (a `can_view` predicate needs something true to key on, not
  just a new global);
- scoot moves to spawning wayvnc itself as session design (the
  session-lock record's "special client launched by the compositor
  itself" half -- a product decision, not a filter);
- a demonstrated local-abuse incident changes the nuisance calculus (to
  date: zero attempts in any probe record; the only virtual-input
  clients on the wire are the real ones).

No fail-first tests: there is no behavior change to pin. What must keep
working -- the switch as default-deny, the lock gate, and the
destroy/disconnect release paths the ticket names -- is already pinned
by the harness suites both halves would break:
`compositor/virtual_input/tests/` (lock refusal, per-device teardown,
flood benches) and the config/reload tests around `[virtual_input]`.
