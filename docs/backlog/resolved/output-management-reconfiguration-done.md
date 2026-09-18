---
title: "`wlr-output-management` reconfiguration — CLOSED (deliberate refusal, kept as a landing spot)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `wlr-output-management` reconfiguration — CLOSED (deliberate refusal, kept as a landing spot).

## The entry as filed

Split out of
[`output-management-read-only-done.md`](./output-management-read-only-done.md)
when the read half shipped (PR #49, 2026-09-16), the same way PR #46 split
the connection-cap item and PR #47 split the wlr foreign-toplevel one — so
what actually shipped and what did not are two separate records, not one
half-true one.

flexwm advertises `zwlr_output_manager_v1` at version 4 and answers every
`zwlr_output_configuration_v1.apply` and `.test` with `failed`. So
`wlr-randr --output <name> --pos 100,100`, `--scale`, `--transform`,
`--custom-mode`, `--mode` and `--off` all print `failed to apply
configuration` and exit non-zero, and a shell's Display page shows an error
rather than a change.

As filed, that refusal was "the right answer *today*" and gated on
multi-output support: flexwm has exactly one `Output`, and the alternative —
a `succeeded` that changed nothing — would give a shell a Display page whose
buttons appear to work (see `compositor/output_management/configuration.rs`'s
module doc for the full reasoning, including why `cancelled` is not used
either). The entry named `set_mode` under `--tty` as the smallest real first
piece, since issue #48 had since landed the mechanism (`State::resize_output`)
on two backends.

## Verify-first findings (2026-09-18, no code)

Probed against `main` at `cbb30ea` before writing anything, taking no outcome
for granted. All four questions came back against implementation:

**1. The write path at the pinned Smithay rev is hand-rolled state
application.** No `output_management` module exists anywhere in `0ff00983` —
re-verified in the vendored source (`src/wayland/` carries
`foreign_toplevel_list` but no `output_management`, and nothing under `src/`
mentions it), not carried over from PR #49's check. The protocol seam itself
is clean (`create_configuration` is the only door in; `refuse` in
`configuration.rs` becomes a real path), but what it routes into does not
exist per bullet below. The read half needs no rework either way.

**2. Applying a mode/position/scale change means something different — and
unsafe — on each backend.**

- `--headless` is the only backend where a caller-chosen size is coherent:
  `resize_output` rebuilds the pixman target and propagates to `wl_output`,
  this protocol, layer surfaces, capture constraints and the layout.
- `--nested` is host-constrained: only the host's *first* configure is acted
  on (`nested_dispatch.rs` acks and ignores every later one), and the
  host-side buffers are sized to match (`nested.rs`'s `apply_size` treats a
  render-target/host-buffer mismatch as a fatal startup condition, and
  `present`'s size guard drops mismatched frames). A client mode other than
  the host window's size cannot be presented; the only honestly-acceptable
  mode is the current size — a no-op success, which is exactly the lying
  success the refusal exists to prevent.
- `--tty` needs a path that does not exist: `hotplug.rs` re-selects from
  what the connector offers and never takes a size it is handed, so a
  client-driven `set_mode` needs caller-chosen-mode application plus refusal
  of anything the connector does not list. The failure *handling* is sound
  (`retarget` allocates buffers before touching DRM state, so a refused
  allocation or a refused `TEST_ONLY` commit leaves the display as it was),
  but a *successful* hostile mode is not a failure: any client could drop a
  4K panel to 640x480 mid-session, and rapid successive applies visibly
  blank the screen each time — a modeset strobe with no rate limit anywhere.
  Custom (unlisted) modes are unhonorable on top of that: drivers reject
  what the connector does not offer.
- `set_position` is meaningless with one output mapped at `(0, 0)`.
- `set_scale` is resolved once from `[output] scale` and fixed for the
  process's life; live rescaling means re-asserting the preferred scale per
  surface on every commit (`output_scale.rs`), which is explicitly not built.
- `disable_head` has no state to land in: every render path, every
  layer-shell map and the core's own `OutputId(1)` assume one output exists.
  Honoring it is an unrecoverable black screen.
- `set_adaptive_sync` has no VRR support on any backend.

The partial-success trap on top: with `disable_head` unhonorable and
nested/custom modes unappliable, `apply` needs atomic all-or-nothing
semantics across backends whose honest answers differ — and the reachable
"success path" (listed modes, `--tty`/`--headless` only) still errors on
`--nested`, which is the primary no-GPU/webtop deployment. A Display page
that works on `--tty` but errors on `--nested` is arguably worse than one
uniform, honest refusal.

**3. The protocol has no authorization concept, and flexwm has no privilege
model to attach one to.** The XML was read, not assumed: the only
"permission" hits are the copyright header, and "should only be used for
output configuration purposes" is a usage note, not an access rule. Any
client can bind today (the bind budget counts it — the least dangerous of
the four capped globals), and `output_management.rs` records why there is no
allow-list: no security-context support (`protocol-gaps-niche.md` still
lists `security-context-v1` as unbuilt). Writes are session-global and
destructive — modeset strobes, an unrecoverable single-output disable —
with no "only the client that…" the protocol would recognize. There is
nothing to restrict writes *to*.

**4. Neither shell needs writes.** DMS's daemon reads and validates profiles
against the advertised list, and its main connection never binds the manager
at all (re-probed 2026-09-18,
[`dms-reprobe-done.md`](./dms-reprobe-done.md)); Noctalia never binds either
and reads `wl_output` (re-probed 2026-09-18,
[`noctalia-reprobe-done.md`](./noctalia-reprobe-done.md)). Zero write
attempts in any probe record. The only writer is `wlr-randr`, run manually
by an operator — and its six refused configurations (recorded in the
read-half entry) already behave honestly.

## Resolution (closed without code, 2026-09-18)

The refusal stands, as an accepted tradeoff rather than a deferred defect —
mirroring
[`connection-cap-denies-the-same-user-done.md`](./connection-cap-denies-the-same-user-done.md).
Not "gated on multi-output" any more: multi-output landing would make
`set_position`/`disable_head` meaningful, but the authorization gap (finding
3) and the per-backend honesty gap (finding 2) close on no backend roadmap,
and there is no client demand (finding 4) to weigh against them.

Revisit if any of these fires, in which case reopen from this record rather
than re-deriving it:

- real multi-output support lands (position and disable become meaningful
  operations, not protocol-shaped holes);
- a real client attempts writes (both probed shells currently read only —
  demand would change the tradeoff, not the mechanism);
- a privilege model lands (`security-context-v1` or equivalent) that gives
  session-global destructive operations something to be restricted to.

No fail-first tests: there is no behavior change to pin. The six-way
`wlr-randr` refusal table in the read-half entry remains the standing wire
evidence that the refusal is what it claims to be.
