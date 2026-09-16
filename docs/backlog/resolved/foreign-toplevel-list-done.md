---
title: "Foreign-toplevel management (window enumeration for external tools) — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Foreign-toplevel management (window enumeration for external tools) — RESOLVED.

## The entry as filed

Foreign-toplevel management (window enumeration for external tools).
User request, 2026-09-13. `ext-workspace-v1` above covers workspaces;
nothing today gives an external client (a taskbar, an alt-tab switcher
applet) the equivalent view of *windows* — only flexwm's own IPC
(`flexwm msg windows`) has that. Check which protocol is actually
current before implementing: `wlr-foreign-toplevel-management-unstable-v1`
is the older, widely-supported one; there may be a newer `ext-` successor
by the time this is picked up, and per `CLAUDE.md`'s standing preference
for the compositor-agnostic successor where one exists, that should win
if it does. No design work done.

## Resolution (2026-09-16, PR #47)

There is a successor, it is `ext-foreign-toplevel-list-v1`, and the pinned
Smithay rev (`0ff00983`) implements it first-class as
`smithay::wayland::foreign_toplevel_list` — while carrying nothing at all
for `wlr-foreign-toplevel-management-unstable-v1`. So the standing rule and
the available implementation agreed, and the entry's "check which is
current" resolves to the `ext-` one with no tradeoff to weigh.

New module `compositor/foreign_toplevel.rs` (plus its test suite), wired to
the three window-lifecycle choke points `shell.rs` already had:
`add_window` announces, `remove_window` closes, `refresh_window` publishes
the title/app id. Nothing new tracks windows; the protocol reads the window
model flexwm already has, which is what makes this list and
`flexwm msg windows`' list the same list by construction.

### The four decisions worth recording

**1. Enumeration only, and that is the whole protocol.**
`ext-foreign-toplevel-list-v1` is "intentionally minimalistic" in its own
words: `identifier`, `title`, `app_id`, `done`, `closed`, and no requests
except `stop`/`destroy`. There is no `activate`, `close`, `minimize`,
geometry or per-output state to decide about — those are left to extension
protocols that do not exist yet. Control stays where it already was:
`flexwm msg action focus-window-id N`.

**2. The identifier is `<generation>-<window id>`** (e.g. `a3e689a2-1`).
The protocol *recommends* an "opaque generation value" and *requires*
non-empty, ASCII, ≤32 bytes, never reused. Eight hex digits of per-session
randomness satisfy the recommendation (flexwm's window ids restart at 1 in
every process, so without it a tool that remembered an identifier across a
restart would match a different window); the decimal suffix is the id
`flexwm msg windows` reports, which is the only bridge from "I enumerated
this window" to "focus it" — the protocol having no requests of its own is
exactly why that bridge matters. Longest possible identifier: 8 + 1 + 20 =
29 bytes, unit-tested at `u64::MAX`, because Smithay's
`new_toplevel_with_identifier` *asserts* the bound and an assert in a
compositor is every client's session.

**3. A handle covers a window's whole life, not the time it is mapped.**
The protocol describes handles as standing for *mapped* toplevels. flexwm
has no map/unmap boundary to hang that on: a window enters the layout, the
focus order and `flexwm msg windows` when its `xdg_toplevel` is created, and
a null-buffer commit does not take it back out. Mirroring flexwm's own
window lifetime keeps the two lists identical; the visible cost is a window
appearing in a taskbar with an empty title for the few milliseconds before
its toolkit sends one, which is what `done` batching is for and is
documented in `README.md` as such. Inventing a map/unmap notion for this
protocol alone would have meant a compositor whose two window lists
disagree.

**4. The list stays live while the session is locked.** Considered
explicitly rather than inherited. Hiding it would mean sending `closed` for
windows that did not close — and then *not* being able to re-announce them
with the same identifiers on unlock, since the protocol forbids reuse — so
a taskbar would be permanently wrong about a session it could not see
anyway. It also would not close a hole: `README.md` already states that
`flexwm msg windows` lists titles while locked, and that a process able to
reach either socket is a same-uid process inside the trust boundary. Pinned
by `the_window_list_stays_live_while_the_session_is_locked`.

### What this does not do — and the finding that matters most

- **It does not light up DMS's or Noctalia's window lists.** Probed rather
  than assumed, and the assumption would have been wrong: stock
  `quickshell` 0.3.1 (the build both probes used) is offered
  `ext_foreign_toplevel_list_v1` by flexwm and **never binds it** — its
  `ToplevelManager` is a `wlr-foreign-toplevel-management-unstable-v1`
  client. With one real `foot` window open it reports `count = 0`. Filed
  with the full measurement as its own entry, and **resolved the same day**
  by PR #50 (`resolved/wlr-foreign-toplevel-management-done.md`): flexwm now
  speaks the wlr protocol too, from the same lifecycle events as this
  module, and the same probe reports the window. That entry also records why
  it was not a small follow-up (no Smithay support, and it is a *control*
  protocol whose minimize/maximize/fullscreen states flexwm's core still has
  no concept of — those requests are accepted and ignored).

  This does not undo the choice of protocol — `CLAUDE.md`'s rule points at
  the `ext-` successor, the pinned Smithay rev implements only that, and it
  is what a standards-following client gets — but it does mean this item
  closed the *standards* gap rather than the two shells' gap, and the
  original entry's motivation ("a taskbar, an alt-tab switcher applet")
  was only half-served until the wlr entry landed.
- **No cap on how many times one client may bind the global.** Each bind
  creates one handle object per window, so a client can multiply its own
  object count. The same unbounded-binds shape as
  `protocols/ext-workspace-object-binding-cap.md`, which now names this
  global too; it stays filed there rather than being fixed here, for the
  same reason it was filed there in the first place.

## Evidence

Compositor code as of **`da38591`** (everything after it is documentation).
Dev VM (`ssh -p 2222 dev@localhost`), debug build, `/mnt/flexwm`.

```
cargo test -p flexwm          TEST EXIT=0    558 passed; 0 failed; 1 ignored
cargo nextest run --workspace NEXTEST EXIT=0 652 tests run: 652 passed, 1 skipped
cargo clippy --workspace --all-targets -- -D warnings   CLIPPY EXIT=0, no warnings
cargo fmt --check --all       clean (run on the Mac; the VM's 9p mount is read-only)
bash scripts/smoke-test.sh    SMOKE EXIT=0   12 "ok:" checks
```

23 of those tests are new (`compositor::foreign_toplevel::tests::*`), all
driving a real `wayland-client` connection through a real `State` and
asserting on the exact event sequence: the zero-window bind, binding before
and after windows exist, one `done` per real change and none for a repeated
`set_title`, `closed` on the right handle when one of two windows goes, a
new identifier after a reopen, the identifier matching the IPC id, two lists
in one client, two clients, a client disconnecting, a handle destroyed
early, `stop`, a handle surviving `stop`, the locked session, and three
bug-bash cases: 200 windows opened and destroyed at the rate a client can
write them (every one announced and closed, nothing left tracked, 200
distinct identifiers), a doubled `stop` followed by `destroy` on a list
whose handles are still held, and a 3000-byte title round-tripping to both
this protocol and the IPC window list.

### Live, against a real compositor and a real client

`wayland-info` against `flexwm --headless`:

```
interface: 'ext_foreign_toplevel_list_v1',               version:  1, name:  7
```

A throwaway `wayland-client` probe (`/var/tmp/ftl-probe` on the dev VM,
not part of the repo) bound before any window existed, with `foot` spawned
over IPC, retitled by its own shell, then closed:

```
registry: ext_foreign_toplevel_list_v1 version 1
-- listening for 16s --
[4278190080] toplevel
[4278190080] identifier=ce5c1f95-1
[4278190080] title=""
[4278190080] app_id=""
[4278190080] done
[4278190080] app_id="foot"
[4278190080] done
[4278190080] title="foot"
[4278190080] done
[4278190080] title="dev@flexwm-vm: ~"
[4278190080] done
[4278190080] closed
```

`flexwm msg windows` at the same moment reported `"id": 1` for that window
— the `-1` suffix of the identifier, which is the documented bridge.

Second scenario, two windows and two probes (one bound before either
existed, one after both did), closing the focused one:

```
--- ids flexwm msg windows reports ---
[{"id":1,"app_id":"foot","title":"dev@flexwm-vm: ~"},{"id":2,"app_id":"foot","title":"dev@flexwm-vm: ~"}]
=== the client bound BEFORE any window existed saw ===
[4278190080] toplevel / identifier=4e08a545-1 / title="" / app_id="" / done
  ... app_id="foot" done, title="foot" done
[4278190081] toplevel / identifier=4e08a545-2 / title="" / app_id="" / done
  ... app_id="foot" done, title="foot" done
[4278190080] title="dev@flexwm-vm: ~" done
[4278190081] title="dev@flexwm-vm: ~" done
[4278190081] closed
=== the client bound AFTER both windows existed saw ===
[4278190080] toplevel / identifier=4e08a545-1 / title="dev@flexwm-vm: ~" / app_id="foot" / done
[4278190081] toplevel / identifier=4e08a545-2 / title="dev@flexwm-vm: ~" / app_id="foot" / done
[4278190081] closed
```

Both clients saw the same identifiers for the same windows, the late one
got the current state in a single batch per window, and the close reached
both. (Transcripts above are the raw probe output, with the repeated
per-field lines of the two initial bursts folded onto one line each for
width; the unfolded form is in the PR description.)

### And the negative result, measured the same way

`quickshell` 0.3.1 with a minimal `ToplevelManager` config, one `foot`
window open:

```
--- flexwm msg windows ---
[{"id":1,"app_id":"foot","title":"dev@flexwm-vm: ~"}]
--- quickshell ---
DEBUG qml: QS: initial count = 0
```

`WAYLAND_DEBUG=1` for the same run: `wl_registry#2.global(7,
"ext_foreign_toplevel_list_v1", 1)` appears 5 times (quickshell's main
registry plus mesa's three) and `grep -c "bind(.*foreign"` is **0** across
the whole 223-line log. See `resolved/wlr-foreign-toplevel-management-done.md`.

The three live scenarios and the quickshell probe were all re-run at
`da38591`, the final code commit, so every transcript here is from the
same tree as the test numbers above. The scripts are on the dev VM
(`/var/tmp/live-probe.sh`, `/var/tmp/live-probe2.sh`, `/var/tmp/qs-probe.sh`,
with the probe crate at `/var/tmp/ftl-probe`), kept so this can be re-run
without rebuilding it.

### Not benchmarked, and why

Nothing this touches is a hot path: the work lands on window open, window
close and a real title/app-id change, never per frame, per input event or
per IPC request. The per-change cost is one `title` (or `app_id`) event and
one `done` per bound list, on top of a `refresh_window` that already runs a
full `arrange` and a configure per window — so an adversarial `set_title`
loop is not newly amplified by this. Window open additionally allocates one
29-byte identifier and one handle.
