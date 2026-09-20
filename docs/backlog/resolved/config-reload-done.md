---
title: "No config reload: every setting is read once at startup — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No config reload: every setting is read once at startup — RESOLVED

## What it said

`README.md`'s "Not yet" list stated it: *"No config reload. Settings are
read once at startup."* Changing a keybinding, a gap or a colour meant
quitting the session — which on `--tty` means losing every client in it.
The ticket scoped the honest shape as partial reload with an explicit
list (gap/colours/focus ring trivial; binds with care around held keys and
the `--tty` VT rule; scale/gpu/renderer probably not), the IPC request as
the cheapest trigger, and failure semantics as the load-bearing half: a
failed reload keeps the running config, never defaults.

## Resolution

Shipped as the ticket scoped it, on the coordinator's four decisions.

**Trigger.** One new top-level IPC request `reload` (`{"type":"reload"}`),
session-level like `windows` — not an `action`. Surfaced as `scootctl
reload` (and `scoot msg reload`) through the shared grammar path, with the
`REQUESTS_HELP`/`USAGE` blocks and the alias parse-equivalence cases
updated alongside. No SIGHUP, no inotify (the ticket's cheapest-first).

**Scope.** `[layout] gap`, `[appearance]` ring width/colors + background +
`prefer_no_csd`, and a full `[binds]` rebuild; `apply()` recomputes the
arrangement and requests a render when anything visible moved (a
binds-only reload skips it). Refused explicitly when they differ:
`layout.column_widths`/`default_column_width` (live columns hold presets
into that list — a shorter list would index out of range in `arrange`, so
the refusal is structural; the gap-only `Config` handed to `set_config`
keeps the running widths, which makes that panic unreachable rather than
merely avoided), the three cursor fields (bitmap/theme built once at
startup), `output.scale`, `tty.gpu`, `renderer.backend`, and
`autostart.commands` (startup-only by nature; never re-run).

**Reply semantics.** `Response::Reloaded { applied, refused }`, both lists
naming only fields that *differed* — two empty lists together mean "the
reload changed nothing it was asked to". Refused entries carry their
reason (`output.scale (startup-only: ...)`). A reload that cannot load or
validate the file answers `Error` (exit non-zero), keeps the running
config, and logs — the file is loaded into a temp `LoadedConfig` and
validated fully before live state is touched.

**Wire version.** The *request* half is additive (an older server meets
`{"type":"reload"}` with a decode error it answers and keeps serving —
pinned by `unknown_request_types_are_rejected` plus `serve`'s
decode-failure arm, error + `Step::Continue`, no kill). The *reply* half
moved `PROTOCOL_VERSION` 2 → 3: a new `Response` variant is exactly the
break-existing-clients change the constant's doc warns about (the
`Warning` 1 → 2 precedent), even though only a client new enough to send
`reload` ever receives one. Read asymmetrically both ways (documented in
`ipc.md`).

**The three pins.**

- *Validate-before-apply:* `a_malformed_file_keeps_the_running_config`
  and `an_unknown_field_fails_the_whole_reload_not_just_the_table` —
  confirmed fail-first (neutered `reload_from` to startup-style fallback:
  all three fail, gap resets).
- *VT guard:* `keybindings_for_keeps_the_vt_recovery_path_unstrippable`
  (pure, no hardware: colliding file bind + `vt=true` keeps `ChangeVt`,
  `vt=false` keeps the user bind) — confirmed fail-first (neutered
  `enforce_vt_binds`: fails). Startup's `tty::init` and reload share the
  one function, so the two cannot disagree. A live `--tty` VT check was
  not run (no VT hardware on the dev VM); stated, not claimed.
- *Held keys:* `a_reload_that_rebinds_a_held_key_keeps_its_release_suppressed`
  and `a_reload_that_binds_a_held_key_forwards_its_release` in
  `input/tests.rs` — release routing follows the press-time
  `suppressed_keys` decision, never the current table, so no wedge and no
  lone release either direction.

**While locked: applies.** Decided deliberately, not by omission: nothing
in the applied set can disclose locked content (appearance only recolors
what the lock screen already shows; gap/binds are input-side, and binds
cannot fire actions while locked anyway). No lock-state unit test exists
— locking needs a real lock client, which this suite does not drive — so
the pin is the absence of a gate in the `Reload` arm next to the `Action`
arm's explicit one, plus the module doc's rationale. A live lock+reload
was not run on the dev VM (no lock client there); stated, not claimed.

**Core touch.** `World::set_config` (validated like `new`, plus
`fix_all_views` since a gap change moves usable edges) — the one
`scoot-core` addition, with its own test module. `match_key` and the
table shape are untouched, so keybind dispatch costs nothing new
per-event; no hot path changed, no benchmark owed beyond that statement.

## Evidence

- `cargo nextest run --workspace`: 1201 passed, 4 skipped (dev VM,
  this branch).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
  `cargo fmt --check --all`: clean. `scripts/smoke-test.sh`: green
  (protocol 3 in the `version` reply).
- Live on the dev VM (`--headless`, `SCOOT_SOCKET` session, foot):
  unchanged reload → two empty lists; gap 12→30 + bg `#101014`→`#ff0000`
  + new `super+n` bind + `scale = 2.0` → applied
  `[layout.gap, appearance.background_color, binds]`, refused
  `[output.scale (startup-only: ...)]`; center pixel `srgba(16,16,20)` →
  `srgba(255,0,0)`; injected `super+n` spawned foot at `x:30,y:30`
  (new gap live); malformed rewrite → loud error, exit 1, gap still 30
  and bg still red; restored file → applied empty, scale still refused;
  repeat → identical (stable).
- Fail-first neuters recorded above; reverted before commit.

## Left out, and why

- **SIGHUP/inotify triggers:** the ticket's separable mechanism 1,
  explicitly cheapest-first. Each refusal-equivalent is a future ticket
  only if someone files it.
- **`column_widths`/`default_column_width` re-application:** refused, not
  rebuilt — existing columns hold presets into that list, and clamping
  every live preset is core surgery beyond this item.
- **Cursor fields:** refused — bitmap/theme built once at startup;
  rebuilding them on the event loop (theme load is filesystem I/O) is its
  own item if wanted.
- **`[output]`/`[tty]`/`[renderer]` re-application, autostart
  re-execution:** refused with messages, per scope. No future tickets
  filed — don't.
- **Live `--tty` VT-switch proof and live lock+reload:** no VT hardware
  and no lock client on the dev VM; mechanism pinned by tests, hardware
  half stated open.
- **Smoke-test section:** not extended — the script's headless run cannot
  cover the VT guard (the load-bearing pin), and the live proof above
  covers the rest. If the script gains a reload section later, it should
  assert the reply lists, not just green startup.
