---
item: "15"
title: "ext-workspace-v1"
status: "done"
area: "protocols"
pr: 24
commit: null
---

# ext-workspace-v1

**No Smithay helper exists for this protocol** at the pinned rev (checked
again: no `ext_workspace` anywhere in `0ff0098`), so
`compositor/ext_workspace.rs` implements the global, the object lifecycle
and the event plumbing directly. The seam is *not* `wayland_server::
Dispatch`: `dispatch.rs` owns a blanket `Dispatch`/`GlobalDispatch` impl
for `State` (item 7's `wl_shm` guard forced it), so a per-interface impl
would overlap it (E0119). The user-data types implement Smithay's
`Dispatch2`/`GlobalDispatch2` instead, which is exactly what that blanket
impl forwards through. Bindings come from `wayland-protocols`' staging
set via Smithay's own re-export — no new dependency, not even a new
feature (Smithay already enables `staging` + `server`).

**A workspace here is a position, not an identity**, because that is what
the core has: an output holds a `Vec` of workspaces, always ending in one
empty one, and leaving an emptied workspace drops it and renumbers
everything after. So the `n`-th handle *is* the `n`-th workspace, named
`"1".."N"` with matching 1-D `coordinates` — `[1]..[N]`, the same 1-based
number, derived once in `describe` so the two cannot drift (a bar sorting
by name alone puts "10" before "2") — and **no `id` event**: the protocol
reserves ids for workspaces stable enough to store preferences against,
which these are not. Worked example of why that is honest rather than a
compromise:
with `[A][empty]` active 0, `move-window-to-workspace down` collapses back
to `[A][empty]` active 0, so nothing is published — and that is correct,
because positionally the user is still on the first workspace.

**Capabilities: `activate`, and nothing else.** `deactivate` (an output
always has exactly one active workspace), `remove`/`create_workspace`
(the layout creates and drops workspaces itself; a user cannot) and
`assign` (one group, because one output) have no meaning in this model,
so none is advertised and each is ignored, which is what the protocol
itself prescribes for an unadvertised request. Group capabilities are
therefore the empty set — still sent, since the event is mandatory once
per object.

**Batching is the protocol's whole point, so it is a pure function.**
`ext_workspace/diff.rs` turns (what clients have been told, what the core
says now) into a change list — `Added`/`Restated`/`Removed` — which the
wire layer executes and closes with exactly one `done`. No `done` at all
when nothing changed, which matters because `State::apply()` runs for far
more than workspace changes (every window open, retitle and layout
action); the cost there is one `Option<Workspaces>` compare. A workspace
leaves its group (`workspace_leave`) before it is `removed`, as the
protocol requires, and a newly active workspace that is also newly
created carries `active` in its first `state` event rather than being
created inactive and immediately restated.

Two exhaustive model tests over the diff (all snapshot pairs up to 6
workspaces) found a real inconsistency that hand-written cases had
missed: from a `published` of `{count: 2, active: 1}` to a `current` of
`{count: 0, active: 0}` — the shape `Workspaces::default()` has before an
output exists — the "new active gains the bit" guard restated a handle
that the same batch was also removing. Fixed by gating it on the
surviving prefix rather than on the old count.

**Requests go the other way with the same batching:** `activate` is
staged and applied on `commit` ("the compositor must process a series of
requests preceding a commit request atomically"), bounds-checked *then*
rather than when it arrived, since the list can change in between — a
client acts on what it last saw, the compositor decides against what it
has. One `Option<usize>` per manager, not a queue: `activate` is the only
staged request and an output has one active workspace, so replaying a
batch in order ends wherever the last one pointed. Activating the
already-active workspace is skipped outright, which is both a no-op in
the core and the thing that stops a client driving a full `apply` (arrange,
a configure per window, a render) as fast as it can write.

**Inertness is derived, not stored.** A handle is live exactly while some
registered manager still lists it; a `removed` one has been truncated out,
one whose manager was `stop`ped or whose client left has no list to be in,
and a destroyed-then-reused object id compares unequal because a
`Weak`'s id carries a serial. All three fall out as "ignore", which is
what the protocol requires of an inert object, with no flag to keep in
sync. Handles and the group are held as `Weak`, so a client destroying
one of its own handles cannot make the compositor address a dead object,
and the slot stays in place rather than being compacted out (compacting
would silently renumber every later workspace).

**One panic path, guarded deliberately:** wayland-backend *panics* when an
event carries an object belonging to a different client
(`rs/server_impl/client.rs`), so the `output_bound` hook — which answers
the protocol's "or a new `wl_output` object is bound by the client" half,
without which a bar that binds the manager before the output sees a group
with no outputs forever — compares `Resource::client()` before sending.
Covered by a two-client test.

**Core changes, both minimal:** `World::workspaces` (count + active read
from one borrow, so the two cannot disagree) and
`Action::FocusWorkspaceIndex`. Activation needs the latter: stepping
cannot express "go to workspace N" when leaving an emptied workspace
renumbers the list. Out of range is ignored rather than clamped —
silently activating the *last* workspace instead would be a switch the
user never asked for. The randomized invariant test now generates that
action too, including `usize::MAX` (it comes off a wire).

**Deliberately deferred, and why:** no IPC action for "focus workspace
N". It would be a `PROTOCOL_VERSION` bump for `flexwm-ipc` on its own;
the backlog already wants one for `OutputSnapshot`'s `usable` rect, and
the two should land together — that backlog entry now carries this as a
clause, so it is picked up from there rather than from here. Also
deferred: per-output workspace groups (one output exists),
`urgent`/`hidden` states (nothing in flexwm marks a window urgent, and
every workspace here is one a user can switch to).

**Verified on the dev VM's real `--tty` seat at 1600x1000** against
`2671b4d` with a clean tree (debug binary for the functional runs,
release for the benchmark; an earlier identical pass ran at `b1ecae7`
and was re-captured after each later commit rather than carried over),
driven by a throwaway `wayland-client` probe in `/tmp` (nothing
committed): `wayland-info` advertises `ext_workspace_manager_v1 v1`; a
client that binds the manager before `wl_output` gets its `output_enter`
in a second `done` batch; a client-driven `activate`+`commit` on the
empty workspace changed **36,336.8** screenshot pixels and switching back
reproduced the original frame exactly (**0** different); 20,000 rounds of
`activate`+`deactivate`+`remove`+two `commit`s took the compositor **191
jiffies over 2.24s** and left it answering IPC normally; `kill -9` on one
of two bound clients left the survivor receiving both subsequent
switches, with no error or panic in the log. Raw output for all of it is
in PR #24's description.

**Re-verified on the same seat against `0273b15`**, after review found
`coordinates` was 0-based while `name` was 1-based and the fix made them
match: the probe reads `name "1"`/`coordinates [1]`, `"2"`/`[2]` and,
with a third workspace open, `"3"`/`[3]`; `activate`+`commit` still
switches (the window goes `visible: false` and the screenshot changes,
`/tmp/ws-fix-empty.png` 33,476 bytes vs `/tmp/ws-fix-back.png` 36,361);
no panic in the compositor log. Nothing else in the protocol behaviour
changed, so the rest of the evidence above still keys to `2671b4d`.

Benchmarked with a map/destroy churn client (400 rounds, every one
changing the workspace list twice — the worst case for this code, not the
quiet one), release builds, `--headless` 1600x1000, compositor CPU in
jiffies. **12 reps with the arm order reversed halfway**, because the
first six showed what looked like a ~5% wall-clock gap and the reversal
proved it was run position: the one 55ms outlier moved to whichever
binary ran first. Pre-change (`e2c7971`) **2.83** jiffies mean /
**115.2ms**, this branch with no client bound **2.75** / **114.8ms** —
indistinguishable. With a bar actually bound (6 reps): **5.67** jiffies /
**178.1ms**, which is the protocol events themselves (an object created
and six events sent per workspace appearing, and a client woken per
batch), not overhead on the quiet path.

**Pre-existing, found while bug-bashing, not fixed here:**
`scripts/smoke-test.sh` under `MODE=--tty` fails its background-colour
check (`the background pixel at (3,3) is rgb(0,0,0)`). It is the *cursor*:
under `--tty` the pointer starts at (0,0) and the built-in arrow is drawn
there, so the sample point lands on the cursor's black outline (a pixel
map of the corner shows the 16x16 arrow). Identical on the merge base
`e2c7971`, so not a regression — see the Backlog entry below.
