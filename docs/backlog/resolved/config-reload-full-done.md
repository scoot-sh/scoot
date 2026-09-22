---
title: "Config reload: from partial to full (live except renderer + DRM device) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Config reload: from partial to full (live except renderer + DRM device) — RESOLVED

RESOLVED 2026-09-22 (PR #TBD): Phase 4 (autostart spawn-delta policy) +
Phases 5–6 (renderer/GPU restart reword), completing the ticket — all
phases landed, end state **live except `renderer.backend` + `tty.gpu`,
which need a restart**. Coordinator-filed, no gh issue, so no `Fixes:`
line applies; the move itself is the close.
Review: pending `scoot-reviewer` (coordinator's gate). Original entry
below, kept verbatim.

> PROGRESS 2026-09-21: Phase 0 + Phase 1 LANDED (PR #209, merge `a279cc0`).
> `apply_reload` split into per-field appliers; cursor theme/size/color
> reload live via `Cursor::rebuild` (per-child `XCURSOR_*` export, render
> without `apply()`). Phase 2 LANDED (PR #211, merge `6481f38`): clamping
> `min(preset, len-1)` in `set_config` after `validated()` (empty→defaults,
> underflow impossible); set-column-width/cycle follow against the new list.
> Review clean both rounds. Remaining: Phase 3 (`output.scale`), Phase 4
> (autostart policy), Phases 5–6 (renderer/GPU reword).
>
> PROGRESS 2026-09-22: Phase 2 LANDED (PR #211). `column_widths` /
> `default_column_width` reload live through one `World::set_config`: a
> shorter list clamps live presets (`min(preset, len-1)`, the existing
> `validated()` clamp precedent — no window silently changes relative
> size), longer lists touch nothing, an emptied list falls back to the
> defaults; `cycle_preset` / `set-column-width` follow against the new
> length, under session lock like gap. Remaining: Phase 3 (`output.scale`),
> Phase 4 (autostart policy), Phases 5–6 (renderer/GPU reword).
>
> PROGRESS 2026-09-22: Phase 3 LANDED (PR #213, merge `06318e4`). `output.scale` reloads live:
> the new value is re-advertised to every output via `set_mode` (bound
> `wl_output` clients hear the new integer) and re-sent to every live
> surface (`preferred_scale` plus the integer companion, walked over every
> window/layer/lock/cursor tree and their popups), every logical geometry is
> recomputed and filed with the core, and `apply()` re-derives the
> arrangement. Out-of-range values apply as their load-clamped selves, never
> as refusals; `--nested` keeps refusing non-1.0 (the host owns the scale);
> framebuffers need no rebuild (physical pixels never moved; the damage
> tracker evaluates geometry at the live scale). Same `Reloaded` variant, no
> `PROTOCOL_VERSION` bump. Review found rescale broke output adjacency vs a
> fresh session — fixed in-round by order-preserving recompaction with a
> fail-first pin; a wire-side (`wl_output.geometry`) position pin was
> suggested and stays open. Remaining: Phase 4 (autostart policy), Phases 5–6
> (renderer/GPU reword).

`resolved/config-reload-done.md` + `resolved/reload-sighup-trigger-done.md`
shipped partial-with-refusal as the honest shape (`Request::Reload` →
`State::reload`, `compositor/reload.rs:92-123` → `config::reload_from`,
`config.rs:448-459` strict, failed reload keeps running config). Live
today: `layout.gap`, appearance ring/background/corner/`prefer_no_csd`,
`binds`. This entry tracks making each refused field live; the target end
state is **live except `renderer.backend` + `tty.gpu`, which need a
restart and get reworded to say so**. Then `README.md:62-66` clears. SIGHUP
inherits every phase free (shared `State::reload()`).

## Phase 0 — groundwork (no behavior change)

Extract per-field appliers out of `apply_reload` (`reload.rs:129-256`)
keeping the pure-compare-first/mutate-after invariant (`:129-130`).
Settle the `startup_*` snapshot rule (`mod.rs:133-134` written once,
diff-only): every phase that starts applying a field advances its
snapshot or compares against the live value — otherwise the second
reload re-reports it (pin `reload/tests.rs:321-333`). Keep the
`{applied, refused}` reply/log shape throughout.

## Phase 1 — cursor rebuild (lowest risk)

`Cursor::new`-once (`cursor.rs:184-247`, built from `state.rs:744-746`,
"deliberately no way to rebuild" `:184-191`) gains a rebuild path:
regenerate `shapes` (`:142-152`), re-resolve `themed` via `refresh_themed`
(`:269-277`), update `size`; also write `self.appearance.cursor_*`
(never written on reload today, unlike ring fields `:154-178`),
re-export `XCURSOR_*` for future children (`mod.rs:265-279`), request a
render (no `apply()`/re-arrange). `Theme::load` (`cursor/theme.rs:105-134`)
never fails — synchronous on the loop is fine. Visible only on `--tty`.

## Phase 2 — column_widths / default_column_width

Refusal is structural (`reload.rs:12-19`, `world/mod.rs:85-92`):
`Column.preset: usize` (`world/tree.rs:39-48`) indexes the list in
`arrange` (`arrange.rs:93-108`); a shorter list indexes OOB (backstop:
`world/tests/reload.rs:80-93`). Pick clamping (`min(preset, len-1)`,
matching `default_column_width`'s existing clamp `config.rs:65-69`) or
proportional remap; `set_config`'s `validated()` + `fix_all_views()`
(`world/mod.rs:93-105`) already does the rest. `default_column_width`
alone (read only at `place_window`, `world/mod.rs:194-200`) can ship
ahead. `cycle_preset` (`world/actions.rs:12`) follows automatically.
Under session lock like gap (no lock-content disclosure,
`reload.rs:46-55`).

## Phase 3 — output.scale (medium risk, all four steps together)

1. Re-advertise via `set_mode` + `smithay_scale(new)`
   (`output_scale.rs:103-109`, `headless.rs:278-304`) — `wl_output.scale`
   re-sent to bound clients.
2. Fractional companions: surface-walk re-send of `set_preferred_scale` +
   `preferred_buffer_scale` (`output_scale.rs:158-197`; today only the
   bind-time moment exists).
3. Layout: recompute logical geometry (`output_scale.rs:138-149`,
   `headless.rs:255-268`), core areas, `fix_all_views`, `apply()` +
   render. Cursor hotspot math already takes scale per frame.
4. Resize per-output framebuffers (`configuration.md:113-123`).
`--nested` keeps refusing non-1.0 (host owns scale, `mod.rs:90-99`).
Per-output scale stays out (milestone 19's surface). Expect churn:
clients that cache scale may lag
(`ghostty-fails-at-1-5-done.md`, `output_scale.rs:170-176`).

## Phase 4 — autostart re-run policy (semantics first)

Mechanism is one line (`state.act`, as `mod.rs:303-305`); the policy is
the work. Recommended: run-only-new-`Spawn`-entries (diff vs
`startup_autostart`, advance snapshot — never full re-drain, never
non-spawn actions; a reloaded `quit` must not kill the session,
`config.rs:1144-1157`). Under lock: skip/defer, unlike gap/binds
(`reload.rs:46-55`).

## Phases 5–6 — renderer.backend + tty.gpu: restart semantics, not live swap

Live renderer swap (rebuild every `Pipeline`, `render.rs:221-245`,
re-import all client textures, scanout-tier framebuffer contract
`:254-267`, non-atomic, no rollback) and live DRM re-open (new device
through libseat, `tty/gpu.rs:19-29,123-145,193-215`, re-pick
connector/mode, rebuild surfaces, drop old master — a restart keeping
clients, every step fallible mid-flight with fail-closed no-display risk)
are both disproportionate. Keep refusing; reword to name restart. No
third reply list (that would bump `PROTOCOL_VERSION` 3→4 per
`scoot-ipc/src/lib.rs:40-47`; migrating strings applied↔refused needs no
bump).

## Tests and docs

Per phase in existing harnesses: cursor rebuild applies + idempotent;
core remap units + `reload/tests.rs` applied-preset assertions;
headless live-rescale (wire `scale` event, fractional re-send, geometry
halving, nested refusal); spawn-delta runs once, non-spawn refused, lock
skips, no double-run. Unmodified pins: `reload/tests.rs`
(validate-before-apply, vanished path, pathless, VT guard),
`input/tests.rs` held-key, `sighup` (10), `world/tests/reload.rs`
backstop. Docs move fields Refused→Applied per phase
(`configuration.md:234-238,276-334` + per-table Startup-only tags,
`reload.rs:10-44`, `response.rs:156-160`, `output_scale.rs:31-45`,
`cursor.rs:184-191`, `config.rs:267-309,567-569`).

Ship order: cursor → `default_column_width`, then `column_widths` →
`output.scale` → autostart policy → reword renderer/GPU.

## What the final PR did (2026-09-22, PR #TBD)

Phase 4, run-only-new-`Spawn`-entries: `apply_autostart_reload`
(`reload.rs`) diffs the fresh list against `startup_autostart` as a
multiset by value (`autostart_delta`, pure and pinned: edited-in-place is
new, removed-then-re-added runs again, duplicates per occurrence), runs
each unseen `Spawn` through the same `state.act` startup drains, refuses
each unseen non-spawn by name (a reloaded `quit` never reaches `act`),
reports the field applied once, and advances the snapshot past the whole
fresh list -- so a second identical reload is silent. Under lock it skips
(a spawned program at lock time could disclose or interfere) *and* freezes
the snapshot: deferred to the first unlocked reload, not dropped; an empty
delta stays silent even under lock. SIGHUP inherits all of it through the
shared `State::reload` (pinned: HUP runs new entries, never re-runs old
ones, skips-while-locked).

Phases 5–6: both refusals reworded to `takes effect on restart: ...`, no
third reply list, `PROTOCOL_VERSION` stays 3 -- strings are payload, and
the wire pin proves an older client parses the new strings as the same two
lists. `README.md`'s reload line now reads "live, except two restart
fields". Reload is a cold path (one file read + diff per request), so no
benchmark. Fail-first: the quit/reword/spawn/lock pins were each proven
red by neutering the policy before it landed green. Live dev-VM proof:
headless session -- new entry runs only itself, second reload silent,
reloaded `quit` refused with the session alive, restart strings in the
reply, HUP shares the delta (runs new, silent on old), remove-then-re-add
and edit-in-place per the diff semantics, rapid double reload converges.
Lock+reload live was impossible on the VM (no lock client installed);
the real-`ext-session-lock-v1` harness tests stand in its place.
