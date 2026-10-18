---
title: "Extract `scoot-dbus`: move the hand-rolled D-Bus client into its own crate"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# Extract `scoot-dbus`

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scoot-dbus-library-cratesscoot-dbus).
Serves **both**: scootnotify serves `org.freedesktop.Notifications`, scootlaunch calls `systemd1`
and scootgreet calls `login1`. None of that is drawing.

## What to do

- Move `crates/scootbar/src/dbus/{mod,conn,proto,link,fuzz}.rs`, their tests and fixtures to
  `crates/scoot-dbus`. The MPRIS and BlueZ readers stay in scootbar. The `dbus` fuzz target and
  corpus move too.
- Its `crate::print::warn` sites (`conn.rs:640,658,766,778,789,797,828,840`;
  `link.rs:353,388`) and `runtime_dir` sites (`conn.rs:111,207`, `link.rs:302`) call `scoot-rt`.
  The hard-coded `scootbar: dbus:` prefix becomes the binary's name.
- **`scoot-dbus` depends on `scoot-rt` and on nothing that draws** (checked with `cargo tree`).
- Add `scoot-dbus` to CLAUDE.md's library scope list, and add its CI arms (fanning out to every
  consumer).

## Acceptance

- scootbar's tray, media, bluetooth and power tests pass unchanged against a real `dbus-daemon`.
- The ratchet's tray and media rows are within noise.
- `Cargo.lock` gains no external crate.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `cargo tree -p scoot-dbus` output.

## Not in this ticket

What scootnotify needs: an error body, a reply to an over-cap call, a per-connection cap.
Those land in `notify-server` with their consumer.

## Resolution

Landed in PR #665 (branch `refactor/scoot-dbus-extract`, against
`origin/main` at `9a6d4dcd`).

A mechanical move with no logic edits (`git mv` plus `pub` and paths;
`git diff -M` shows every moved file as a rename):

- `crates/scoot-dbus` (new): `conn`, `proto`, `link`, `fuzz` with their
  tests (`tests`, `daemon_tests`, `link/tests`, `fuzz/tests`), fixtures
  and `testdaemon`. `#![forbid(unsafe_code)]`, Linux-only like scootbar.
  Depends on `scoot-rt` and `rustix` only.
- `crates/scootbar`: `pub use scoot_dbus as dbus`, so every
  `crate::dbus::…` path (modules, tests, fakes) resolves unchanged.
- Fuzz: the `dbus` target, its `#[path]` harness, the seed corpus and
  `regressions/` move to `crates/scoot-dbus/fuzz` (own workspace and
  lock); scootbar's fuzz workspace loses only that target.
- CI: classify arms for `crates/scoot-dbus/Cargo.toml` (also `scoot`)
  and `crates/scoot-dbus/*`, fanning out to `scootbar` (the only
  consumer; no always-run job). The scootbar job now runs fmt, the
  clippy matrix, both test runners and the fixed-budget dbus fuzz over
  the new crate; macOS excludes it beside scootbar/scootui.
- `CLAUDE.md`: scope list gains `scoot-dbus`.

Deviations from the ticket's file list, all mechanical:

- `mpris.rs` and `bluez.rs` (with their `build`/`tests` helpers) move
  too. `fuzz.rs`, which moves per the ticket, walks those readers over
  every input (`use super::bluez/mpris`); leaving them behind would
  have split the "written once" check or duplicated them. They use
  `std` and `proto` only, so the leaf keeps its dependency posture,
  and scootbar's `crate::dbus::{bluez,mpris}` paths still resolve.
- The `scootbar: dbus:` / `scootbar: {who}:` prefixes become the
  running binary's name, read once from `argv[0]` (`prog()`); under
  `scootbar` the lines are byte-identical to before.
- Made available to dependents (not `cfg(test)` only), since the apps'
  tests use them cross-crate: `bluez::build`, `mpris::build`,
  `Writer::with_cap`, `Writer::begin_signal_to`, `Conn::queue_raw`,
  `Addr::Stream` (with its `start_on` arm), and the always-compiled
  `testdaemon` (like `scootui`'s `testfont`). The fuzz crate's `#[path]`
  include resolves `build` through an explicit path attribute.
- The link carries its quick-death retry wait as a field
  (`start_with_retry`/`start_on_with_retry`); production constructors
  delegate with the unchanged 30 s, and the two tray latch tests hand
  over 300 ms through a test-only helper. The old `#[cfg(test)]` 300 ms
  constant no longer reaches consumers' tests now the link lives in
  another crate.
- `clippy::result_unit_err` is allowed at the crate root: the `Err(())`
  refusal idiom is deliberate, and re-typing every reader would be a
  redesign, not this move. (The lint never fired in the binary crate,
  where `pub` is not externally reachable.)
- scootbar's `control::paths::runtime_dir` gate narrows to
  `volume`/`microphone`: the tray, media and D-Bus client read the same
  directory through `scoot_dbus` now.
- `Writer::with_cap` doc, the `build` helpers' docs and the fuzz/doc
  path mentions point at the new homes. `SCOOTBAR_REQUIRE_DBUS_DAEMON`
  keeps its name (CI sets it; renaming is behavior change, out of
  scope). `site/` needs nothing (no path mentions; the "bar's own
  client" lines still describe behavior).

Evidence (dev VM, aarch64, rustc 1.97.1,
`CARGO_TARGET_DIR=/dev/shm/w13-target`, `SCOOTBAR_REQUIRE_DBUS_DAEMON=1`):

- `cargo nextest run -p scoot-dbus`: 78 passed. `cargo test
  -p scoot-dbus`: 78 passed (opposite runner).
- `cargo nextest run -p scootbar`: 1294 passed, 4 skipped.
  `cargo test -p scootbar`: all suites ok.
- Feature matrix: `--all-features` 1138 passed; `--no-default-features`
  414 passed; each of the 18 modules alone 414–499 passed.
  `scootui`/`scoot-rt` default 131, no-default 45, each alone 75–99.
- Snapshots: 17 passed, no re-bless. Allocation pins: scootbar 8
  passed; scoot-dbus's signal/idle pin passes (the counting allocator
  moved with the tests).
- Clippy `-D warnings`: default, `--no-default-features`, each of the
  18 modules alone, `popup`+each of the other 17, scootui default /
  no-default / each alone, `scoot-dbus`. All green.
- `cargo fmt --all --check`: clean.
- Fuzz: both fuzz workspaces `cargo check --locked` clean; `dbus` at
  the CI budget (`-runs=1000000 -seed=1 -timeout=10 -max_len=70000`):
  1,000,000 runs in 43 s, no findings, no artifacts.
- `cargo tree -p scoot-dbus -e normal`: `scoot-dbus`, `scoot-rt`,
  `rustix`, `linux-raw-sys`, `bitflags`. No `libc`, nothing that draws.
- `Cargo.lock`: gains only the `scoot-dbus` path entry (on the existing
  `rustix`/`scoot-rt`/`scootbg-mem` lock entries) and scootbar's edge
  to it. No external crate.
- Ratchet (release builds, `main` at `9a6d4dcd` vs branch at
  `5b8c93a03`, solid bar `--center=` with DejaVu under headless scoot
  from the branch tree, fresh XDG dirs, 14 s settle, 60 s window):
  stripped size 3019488 vs 3019520 (+32 B); `.text` 2444520 vs 2446536
  (+2016 B, +0.08%; corrected in review from the hex 0x254ce8 vs 0x2554c8); idle RSS 4084/4084 vs 4020/4020, PSS 3198/3121 vs
  3062/3066, wakeups Δ0 vs Δ0, fds 7 vs 7. Within one-run noise; no
  waiver needed.
- `git diff --name-only` has no `scootbg`.
- `git diff -M --stat` shows the moves as renames.
