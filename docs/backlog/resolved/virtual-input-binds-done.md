---
title: "Opt-in keybindings for virtual keyboards"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Opt-in keybindings for virtual keyboards

Filed 2026-10-07. Serves **computer use** (a webtop/VNC/try-scoot session whose only user is remote is half a demo when no bind fires) and **daily-drive** (remote admin of a real session).

## The gap

The try-scoot demo (headless scoot + wayvnc + noVNC, scripted RFB client proven live) types and clicks, but no keybinding ever fires over VNC (`Alt+Return`, `Alt+d`, `Super+Return`): `crates/scoot/src/compositor/virtual_input.rs` delivers virtual keys forward-only by design ("window management stays local"; `site/src/content/docs/scoot/remote-desktop.md`). Right default for a local user plus remote helper, wrong for a session the remote user owns.

## What to do

Add explicit opt-in, default off, `[virtual_input] binds = true`: virtual-keyboard keys run compositor binds by translated seat keysym. Pin: layout-mismatch translation, held-modifier + `release_source`/destroy/lock/VT (no stuck binds/modifiers), lock gate absolute (never unlock, never bind while locked), no bind repeat for virtual (fire once), no double-trigger across sources (Smithay per-source absorb), focus-follows, default-off inert (bool check only), restart-only reload (like `enabled`). Docs: config reference + `remote-desktop.md` (trust note, webtop recipe), `CHANGELOG`. Tests: headless protocol tests (on/off/locked/mismatch/stuck) + live wayvnc+RFB on M2.

## Not in this ticket

Per-client allow-list (`docs/backlog/protocols/virtual-input-allow-list.md`); virtual-pointer binds (pointers have none); live reload of the flag.

## Resolution (2026-10-07, PR #494)

Landed as `[virtual_input] binds` (default off, restart-only with `enabled`).
`crates/scoot/src/compositor/virtual_input.rs`: bind-aware `input_from_source`
filter (seat-keysym matching, fire-once, absolute lock gate), separate
`State::virtual_suppressed` set, press-time `held` mirror per device, teardown
synthesizes releases through the filter when on and `release_source` when off.
`config.rs`/`state.rs`/`mod.rs`/`reload.rs`: the knob, the live bool, startup
wiring, restart-only refusal as `virtual_input.binds`.

Evidence: 10 new headless protocol tests (binds on/off, de-keymap Alt+Return
and Super+z/y, locked-neither, destroy/lock sweep swallowing, exactly-once,
no-repeat) + reload refusal test + config parse/emission tests -- the firing
tests fail under a forward-only revert (proven); `cargo nextest run
--workspace` 4454 passed, `cargo test -p scoot` 2199+4 passed, clippy/fmt
clean, `scripts/smoke-test.sh` rc=0, `nix build .#docs-site` exit 0. Live on
the M2 (headless scoot + wayvnc 0.10.2 + scripted RFB Super+Return): binds on
opens foot, binds off opens nothing with the compositor alive. Docs:
`remote-desktop.md`, `configure.md`, `troubleshooting.md`, CHANGELOG.
