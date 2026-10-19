---
title: "scootlaunch: the dmenu contract (fuzzel's flags the call sites use) and run mode"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootlaunch: dmenu and run

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#the-dmenu-contract). Serves
**daily-drive**: the bar's WiFi and Bluetooth pickers, and the clipboard and capture choosers.

## What to do

- `--dmenu` with every flag in the design's table, at fuzzel's meaning.
- `--index` and `--placeholder`.
- The stdin bounds.
- Exit 1 on Escape.
- A loud exit 2 on any unsupported flag (Decisions needed (c)).
- `--run` / `--list-executables-in-path`.

## Acceptance

- Each repo call site (`apps-network-pick.sh`, `apps-bluetooth-pick.sh`, the `keys-home.nix`
  clipboard, `capture-home.nix`) runs against scootlaunch in a headless test with the same
  flags.
- The WiFi password prompt draws masked.
- A 65,536-line stdin stays bounded.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/cli.md` with `--dmenu` flags, stdin/stdout contract and exit codes, with a runnable snippet. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Swapping the call sites in Nix (`launch-nix`).

## Resolution

Landed 2026-10-19 in PR #678 (`feat(scootlaunch): dmenu and run
modes`), on `feat/launch-dmenu`.

What shipped: `src/cli.rs` (the `--dmenu`/`--run` commands, `=` and
space value forms, loud exit 2 on anything else), `src/dmenu.rs` (the
bounded stdin read, the ranked state machine reusing the matcher's
`Ranker` with no frecency, the masked password entry over a leaked
1 KiB caller buffer wiped on drop), `src/dmenu/lines.rs` (the
terminator-aware splitter: 1 MiB, 65,536 lines, 4 KiB per line),
`src/run.rs` (the `$PATH` scan: first directory wins, symlinks
followed, 65,536 names), `src/panel.rs` (the dmenu paint with the
prompt at `Geometry::text_x()` and the `*` mask, `--index` printing,
per-invocation color overrides), `tests/dmenu.rs` (every repo call
site's flags, the masked prompt, `--no-run-if-empty`, Escape, the
65,536-line bound, run mode), the `dmenu` fuzz target wired into the
scootlaunch CI job, and `site/.../scootlaunch/cli.md` rewritten with
the flags, the stdin/stdout contract, exit codes and a runnable
snippet (plus `index.md`/`panel.md` deltas).
