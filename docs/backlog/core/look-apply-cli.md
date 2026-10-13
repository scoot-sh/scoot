---
title: "scoot look apply: the atomic swap, ordered reloads, rollback, list/current/next/prev/validate"
status: "open"
area: "core"
priority: "high"
blocked: "look-layer-scoot, look-layer-scootbar"
---

# scoot look apply: the atomic swap, ordered reloads, rollback, list/current/next/prev/validate

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
this is the swap itself, and the end of the first slice.

## The gap

Nothing switches a look after evaluation. The pieces each app needs exist
(scoot `Request::Reload`, `scoot-ipc/src/request.rs:115-122`; scootbar's
control-socket `reload`, `crates/scootbar/src/control/protocol.rs:1-17`;
scootbg through scoot), but nothing renders a look, points the apps at it,
or reloads them together.

## What to do

`scoot look` as a subcommand of the existing CLI (the design doc's §7.1):

- `list`, `current`, `show`, `validate`, `apply NAME [--for SECONDS]`,
  `next`, `prev`, `random`, `build DIR --out OUT`; `--json` on every one;
  exit codes in the `scoot msg` style.
- The catalog: `$XDG_DATA_HOME/scoot/looks/*` plus the shipped looks the
  binary knows the path of. The render cache under
  `$XDG_STATE_HOME/scoot/rendered/<name>-<hash>/`.
- The transaction of §4.3: render and validate everything (font
  resolution and the wallpaper decode cached by path, size and mtime) →
  `symlink` + `rename` the link → reload scoot (waiting for the wallpaper,
  see [`look-layer-scoot`](look-layer-scoot.md)) → reload every running
  scootbar instance → on any first-party refusal, repoint back, re-reload
  what already ran, exit non-zero naming the app.
- The reply (and `scoot look current`) lists every key in the user's own
  files that shadows a look token, by name (design doc §4.3 step 6): an
  install that `install.sh --look` seeded has the look's colors in its own
  `config.toml` and `bar.toml` (`site/public/install.sh:446,460`), so
  without this a swap would answer "applied" and change nothing visible.
- `--for` reverts after the timer unless `scoot look apply --keep` arrives
  (a preview), surviving the terminal closing (a detached child or a user
  timer).
- A lock file so two swaps never interleave.
- Docs in the same PR: a new site page for looks under
  `site/src/content/docs/scoot/` (task-first: pick, preview, keep), and the
  `--help` text through the generated CLI pages.

## Evidence to record

Tests: atomicity (a reader racing a swap sees the old or the new directory,
never a mix), refuse-and-keep (a broken look leaves every app on the
previous one), rollback (a scootbar refusal after the repoint restores the
link and re-reloads scoot), a dangling link, a missing compositor (refused
with a clear message, nothing changed), a shadowed key reported by name
rather than silently winning. Benchmark on headless and on
`--tty`: swap wall time, the compositor's frame times across a swap (no new
dropped frames beyond what `apply()` costs today), and zero new per-frame
allocations after it. Live: the smoke test applies two looks and checks the
background pixel of each.

## Not in this ticket

IPC, Nix, third-party apps, authoring commands.

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
