---
title: "scootlaunch: the panel, keys, the virtualized list, the look layer"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# scootlaunch: the panel

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#keyboard-first-ux); split in the #579 fix
round. Serves **daily-drive**. `look-format` is the live-looks `scoot-look` crate (#578, merged).

## What to do

- The `overlay` exclusive-keyboard panel with the entry and the virtualized list.
- Keys and mouse per the design.
- The single-instance toggle over a socket file in `$XDG_RUNTIME_DIR` (`0700`), with the peer's uid checked through rustix's `socket_peercred`. There is no pid file and no abstract name another user could bind first.
- A null output: the core ticket `layer-shell-default-output` decides which output that is.
- The look layer, read at every open: `<state>/look/launch.toml`, the allowlist and the grammar.

## Acceptance

Headless tests:

- open, type through `scoot msg type`, and select;
- the toggle closes a running instance;
- goldens per shipped look against today's fuzzel colors (the design's table);
- zero allocation per keystroke.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Launching (`launch-spawn`), icons (`launch-icons`).

## Resolution

Shipped in PR #674 (`feat/launch-ui` branch): the overlay
exclusive-keyboard panel, the `launch.toml` look fragment, and the
CI/docs that go with them.

What landed:

- `scoot-look render_launch` plus `scoot-look build` and the apply /
  build / cache / sync paths (`RENDER_VERSION` 11): every render
  carries `launch.toml` with the thirteen shell-app roles, the UI font
  at its popup size, the popups opacity and the look radius, with
  `[apps.scootlaunch]` overrides merging per key.
- `scootlaunch` panel (bare invocation): null-output overlay surface
  with exclusive keyboard (scootui `surfaces` kind), centered, 40 em
  wide with 10 rows; `Entry` plus the matcher-fed virtualized `List`
  with greedy match-highlight spans; the design's keys (Up/Down,
  Ctrl+P/N, Tab/Shift+Tab, PageUp/Down, Ctrl+Home/End, Enter,
  Ctrl+Enter, Escape; the entry keeps its editing keys); click picks,
  wheel scrolls; the single-instance toggle over
  `$XDG_RUNTIME_DIR/scootlaunch-<display>.sock` with peer-uid checks;
  the look layer read at every open through the allowlist and grammar;
  `--font` / `--config` / `--check`. Picking prints the id (exit 0),
  dismissing exits 1; no surface is held past close.
- Headless tests (`crates/scootlaunch/tests/panel.rs`, fail instead of
  skip under `SCOOTLAUNCH_REQUIRE_SCOOT`): open, type through
  `scoot msg type`, redraw asserted on screen, select prints the id;
  Ctrl+N through real modifiers events; toggle closes the first panel;
  Escape dismisses; row click picks; pointer motion outside does not
  dismiss; goldens per shipped look (border-box geometry, surface fill
  blended at the look opacity, selection fill) with the layer rendered
  by the real `scoot-look build`.
- Goldens per shipped look against today's fuzzel colors (the design's
  table): `look/tests.rs` renders `launch.toml` and `fuzzel.ini` for
  all five looks and pins all seven role pairs, then checks the
  panel's own read agrees.
- Zero allocation per keystroke: `Ui` reuses every buffer (rank,
  collapse, spans, highlight frame on the stack); pinned by
  `keystrokes_allocate_nothing_after_warm_up` and mutation-proven (a
  `format!` in the rerank goes red, reverted green).
- Fuzz: the `fuzzy` target additionally pins spans-agree-with-score.

Evidence (dev VM, aarch64, rustc 1.97.1; exact heads in the PR):
`cargo nextest run -p scootlaunch` 199 passed; `cargo test
-p scootlaunch` and `-p scoot-look` green on the opposite runner;
`scoot-look` 182 passed; `scootctl look` 257 passed both runners;
`scootui` + `scoot-rt` 188 passed; `cargo clippy -- -D warnings`
(scootlaunch, scootctl, scoot-look, scootui) and `cargo fmt --check`
clean; the release binary links only glibc, libgcc_s and
libxkbcommon; fuzz workspaces check clean (the `dead_code` warnings
are the pre-existing subset kind). Screenshots per look with a query
typed in `scratchpad/W20-shots/`; moonrise and music-desk committed
under `docs/assets/` and shown on the new site panel page.

Known deviations (stated, not silent):

- A click on the bare desktop does not dismiss in v1: with exclusive
  keyboard the compositor holds focus (`layer_keyboard_focus` keeps an
  exclusive surface first), so no leave arrives and the click goes to
  the surface below. A keyboard leave with the pointer outside still
  dismisses (lockers keep drawing and wait, per the failure table).
- The caret/focus ring follow `selection` when a look overrides it
  apart from `accent`, and the selected ink follows `background` when
  `selection-text` is overridden apart; the defaults coincide and no
  shipped look overrides either apart (goldens pin all five).
- Rows show the entry name only (generic/comment subtitles deferred);
  the look `font` family is validated but the file comes from `--font`
  or the well-known paths (no fontconfig); Ctrl+Enter prints the typed
  text as the run-mode stand-in until `launch-spawn`.
- Null output: `layer-shell-default-output` has not landed, so the
  panel opens on the compositor's primary (today's behavior for mako
  and fuzzel too); the panel carries no output logic of its own.
