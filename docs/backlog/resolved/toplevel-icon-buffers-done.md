---
title: "`xdg-toplevel-icon-v1` pixel-buffer icons are accepted but not exposed — DONE (name-only stands, verified)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `xdg-toplevel-icon-v1` pixel-buffer icons — DONE (name-only stands, verified)

Resolved as a verify-first close: every half of the ticket's suspicion was
checked against the pinned sources, and each one came back "no bug, no
leak, nothing to release". What ships is four pinning tests plus this
record; no compositor behavior changes, no new IPC field, no new
advertisement. Name-only exposure stands as the honest scope until a real
consumer asks for pixels -- the ticket's own exit condition, still unmet.

## What the ticket feared, and what the sources say

**"Does flexwm retain icon buffers without release (fd leak)?"** No --
there is nothing to release, by protocol design. The `add_buffer` XML
(`xdg-toplevel-icon-v1.xml`, as vendored in `wayland-protocols` 0.32.13)
says it outright: *"The wl_buffer.release event is unused."* Icon buffers
are never committed to a surface, so the release event -- which belongs to
the surface-commit lifecycle -- never applies. Lifetime is client-owned:
*"The wl_buffer must be kept alive for as long as the
`xdg_toplevel_icon` it is associated with is not destroyed, otherwise a
`no_buffer` error is raised."* Smithay implements exactly that at the
pinned rev (`0ff00983`, `src/wayland/xdg_toplevel_icon.rs`): `add_buffer`
registers a shm destruction hook (lines 389-397) that posts `NoBuffer`
(code 3) when the buffer dies first, killing only that client; destroying
the icon unregisters every hook (lines 405-407). No `release` call exists
anywhere in the module. flexwm itself retains nothing of its own
(`toplevel_icon.rs` stores no per-window copy; the icon lives in the
surface's double-buffered state and is read on demand), so there is no
flexwm-side retention to bound and no flexwm-side release to forget.

**"Do icon buffers count against the live-buffer bound?"** Yes --
verified in code, now pinned in tests. An icon buffer is an ordinary
`wl_buffer` minted by `wl_shm_pool.create_buffer`; `add_buffer` only
*takes* an already-created buffer, it is not a factory. The budget guard
(`dispatch.rs::reject_excess_buffer`, lines 773-784) claims on every
`create_buffer` before delegation, use-agnostically -- it cannot know or
care whether the buffer ends up on a surface, in an icon, or nowhere.
Release (`forget_destroyed_buffer`, lines 829-838) fires on every
`wl_buffer` destruction including disconnect cleanup. The one wrinkle the
module docs already own: an icon holds `WlBuffer` clones server-side, so
the question "do those clones keep dead objects counted?" is real -- and
is now answered by test, not by reasoning (see below).

**"Should pixels be exposed to IPC / foreign-toplevel?"** Foreign-toplevel
is unbuildable without inventing protocol: neither window-list protocol
carries icons at all. `ext-foreign-toplevel-list-v1` events are
`toplevel`/`finished`/`closed`/`done`/`title`/`app_id`/`identifier`;
`zwlr_foreign_toplevel_manager_v1` events are
`toplevel`/`finished`/`title`/`app_id`/`output_enter`/`output_leave`/`state`/`done`/`closed`/`parent`
(checked against the vendored XML, not recalled). Putting pixels where no
standard event exists would be a bespoke extension -- refused by the
standing standards-first rule. IPC exposure is buildable (base64 PNG on
the `windows` reply) but has no consumer: no bar reads IPC window lists
today, no agent request asks for visual recognition, and the cost is on
the IPC dispatch path -- a per-query PNG encode plus base64 on a path
`CLAUDE.md` holds to no heap allocation. The ticket itself gated the work
on demand ("worth doing only once something actually wants it"); nothing
has asked since it was filed.

## What shipped (tests only, no behavior change)

Four tests in `compositor/toplevel_icon/tests.rs`, each confirmed to fail
with its property neutered (see Evidence):

- `icon_buffers_are_counted_in_the_live_buffer_budget` -- three
  pixels-only icon buffers read back as three live units; destroying
  their icons leaves the count at three (buffers outlive the icons that
  named them); destroying the buffers then drains to zero with the client
  still alive, which also pins that icon destruction unregisters
  upstream's hooks (otherwise the destroys would `NoBuffer`-kill).
- `destroying_a_live_icon_buffer_disconnects_only_that_client` --
  destroying a buffer under a committed icon kills that client (upstream
  `NoBuffer`) while a freshly inserted client is still served. The
  load-bearing assertion is compositor survival, the way the frozen-icon
  tests frame it.
- `icon_buffers_drain_when_their_client_disconnects` -- five icon
  buffers, clean disconnect, count reaches zero: the server-side
  `WlBuffer` clones in `IconData` do not pin dead objects or their fds.
- `icon_buffers_fill_the_same_live_buffer_budget` -- 512 pixels-only
  icon buffers (per-buffer pools, the retention shape the budget exists
  to catch, in 64-step chunks so one step never holds more than 64
  client files) fill the budget exactly; the 513rd creation is refused
  and the kill drains the whole count. The single-pool shortcut was
  deliberately not taken: pool topology is irrelevant to the scalar
  count, but the per-buffer shape is what the bound is for.

Test-harness note: `square_shm_buffer` returns the pool backing `File`
alongside the buffer and each step holds it past its roundtrip (the shape
`dispatch/tests.rs`'s `BufferClient` already uses) as defense-in-depth.
PR #100 review verified against the locked wayland-backend source that the
fd is dup'd into the out-queue at request-send time, not at flush — so an
early drop was never actually buggy, and the identical early-drop pattern
in `cursor/tests.rs` is equally harmless. The `File` is kept regardless.

## Evidence

- `XDG_RUNTIME_DIR=/tmp/xdgrt cargo test -p flexwm toplevel_icon` on the
  dev VM: 13 passed (9 pre-existing + 4 new), 0 failed.
- Fail-first, same VM, each reverted before the next:
  - `claim_buffer_creation` stubbed to claim nothing: the three
    counting tests fail, the kill test passes (kill path needs no
    count) -- as predicted.
  - `forget_buffer` stubbed to release nothing: the same three fail
    (count never drains), the kill test passes -- as predicted.
  - T2's destroy replaced with a no-op commit: the client survives, the
    kill assertion fails -- the test discriminates kill from no-kill.
- `cargo clippy -p flexwm --all-targets -- -D warnings` and
  `cargo fmt --check -p flexwm`: clean (same VM, same tree).
- Full workspace set + smoke test: rerun at PR time per the cycle (see
  the PR description for SHAs and raw output).

## What this deliberately leaves open (revisit conditions)

- **Pixel exposure over IPC**, if a bar-over-IPC or an agent visual
  need actually arrives: the ticket's recipe stands (closest-size pick
  from `ToplevelIconCachedState::buffers`, `screenshot.rs`-style encode,
  optional base64 field, rate-limited) -- plus one constraint found
  since: encoding on the IPC dispatch path breaks the no-allocation bar,
  so it wants the screenshot worker or an explicit carve-out, not an
  inline encode.
- **Nothing on foreign-toplevel** unless a standard protocol grows an
  icon event; bespoke icon events are out of scope by the
  standards-first rule.
- No hot-path benchmark: nothing on a hot path changed (tests +
  docs only). Stated so the absence is not mistaken for an omission.

## Bookkeeping

- Open ticket `docs/backlog/protocols/toplevel-icon-buffers.md` removed
  (this record replaces it).
- `docs/backlog/README.md` index entry repointed here.
- `README.md` "Window icons" section carries the buffer half it
  previously summarized in one clause: buffers accepted (square shm),
  counted in the 512 live-buffer budget, early destroy disconnects that
  client, pixels never leave the compositor.
- `toplevel_icon.rs` module doc's "gap this leaves" now points here
  instead of at the open backlog.
