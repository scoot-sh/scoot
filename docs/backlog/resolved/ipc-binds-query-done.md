---
title: "Live keymap query (scoot msg binds), default Super+? bind, and explicit unbind"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Live keymap query (scoot msg binds), default Super+? bind, and explicit unbind

Filed 2026-10-06 (implementer brief `binds.brief.md`). Serves **daily-drive and
computer use**: a user who hits a dead key or a forgotten bind needs the live
keymap in one keystroke, and an agent driving the session needs the same map
as JSON without re-parsing the user's config file.

## The gap

- There is no way to ask the running compositor what keys do what. Config
  `binds` are additive per-combo overrides over the defaults (removing a line
  falls back to the default), but **there is no unbind** — a default bind
  cannot be removed except by rebinding the combo (see
  `site/src/content/docs/scoot/keybindings.md`: "There is no unbind
  action"). Parse errors, colliding groups and unbound key names only go to
  the log (`crates/scoot/src/compositor/` keymap/config code paths).
- No default bind shows the keymap; `Super+h` is taken (focus column left),
  so the brief proposes `Super+Shift+/` (Super+?, niri's hotkey-overlay
  convention) pending a freedom check against the default keymap.

## What to do

1. `scoot msg binds`: a new IPC request listing the **effective** keymap the
   running compositor uses (live merged result, not a re-parse of the file).
   Each row: combo in canonical spelling, action string, source (`default`,
   `config`, or `config (replaces default: <old action>)`), plus `repeat`
   and `allow_when_locked` flags. A section for skipped config binds, each
   with its reason. Human output: grouped, aligned, 80-column table;
   JSON via the existing `--json` conventions. Protocol version bump per
   `scoot-ipc` rules (see how #463 added `locked`); single-source help
   (`crates/scootctl/src/help.rs`), `--help --json` contract, site IPC
   page. No allocation on the IPC dispatch hot path beyond the reply.
2. Default bind `Super+Shift+/` (or closest free combo, with rationale):
   spawn the user's terminal running `scoot msg binds` in a pager, reusing
   the terminal `Super+Return` spawns unless a dedicated action is cleaner.
   Must work fullscreen-focused, must not fire while locked.
3. Unbind: a `[binds]` value removing a default (e.g. `"super+h" = "none"`),
   shown as `config (unbinds default: <old action>)`. Spelling chosen
   against the action grammar so `none` cannot collide; documented on the
   keybindings page. Purely additive, not breaking. Check whether
   `nix/modules/keys-home.nix` should use it.

Tests (fail before, pass after): IPC `binds` reply for a known config
(default / override / unbind / skipped-with-reasons rows); the default
bind exists and spawns; the unbind removes the default and stops it
firing; help single-source tests extended.

## What landed (2026-10-06, PR #479)

All three, on branch `feat/scoot-msg-binds`:

1. `scoot msg binds` / `binds --json`: new `Request::Binds` /
   `Response::Binds{bindings, skipped}` (protocol 8 → 9). Rows derived by
   diffing the live table against the defaults per request (no stored
   provenance); skipped entries collected in `apply_binds` (previously
   log-only) and swapped on reload. Single-source help extended
   (`REQUESTS` row, `binds [--json]` in `REQUESTS_HELP`, `verb_text`
   works so `help binds` resolves).
2. Default `Super+Shift+/` (`super+shift+slash`, verified free in the
   default map and the desktop profile): a dedicated `show-keymap` action
   (core → `Effect::Spawn(["foot", "sh", "-c", "scoot msg binds |
   less"])`), chosen over a `Spawn` default because the pager pipeline has
   no `[binds]`-grammar spelling that parses back past the
   `--print-default-config` round-trip pin. Same `foot` `Super+Return`
   spawns (pinned by test). Fires fullscreen-focused (no fullscreen gate
   on binds, only the lock gate); never fires locked (not a `Spawn`, so
   the allow-list refuses it; `allow_when_locked` clamped at load, repeat
   refused by the backstop).
3. Unbind: `"combo" = "none"` (string or `{ action = "none" }` table;
   `none` is no action verb, so no collision). Participates in collision
   groups; an unbind removing nothing warns and lands in skipped.
   `nix/modules/keys-home.nix` needs nothing: it only adds fresh combos,
   overriding no default.

Evidence: `cargo nextest run --workspace` 4397 passed / 0 failed;
`cargo test -p scoot` 2155 passed; clippy/fmt clean;
`scripts/smoke-test.sh` rc=0; `nix build .#docs-site` green; live
headless session verified the human table, `--json`, reload-applied
`binds`, the default bind spawning `foot sh -c 'scoot msg binds | less'`
plain and fullscreen-focused, the unbind holding, and the override
firing. Fail-before proven by revert-run-restore (unbind recognition
off: 5 fail; gutted snapshot: 10 fail).

## Not in this ticket

- An on-screen overlay instead of a terminal (only if trivially small; no
  new surface role).
- Changing any bind in `nix/modules/keys-home.nix` beyond reporting
  whether it should use unbind.
