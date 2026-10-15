---
title: "scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the swap itself, and the end of the minimum slice.

## The gap

Nothing switches a look after evaluation. Each app's reload exists (scoot
`Request::Reload`, `scoot-ipc/src/request.rs:115-122`; scootbar's control
socket, `crates/scootbar/src/control/protocol.rs:1-17`), but nothing renders
a look, points the apps at it, or reloads them together.

## What to do

`scoot look list`, `current`, `show`, `validate`, `build`, `apply NAME`,
each with `--json`, per the design doc's sections 3.1, 4 and 7.1:

- The catalog: source looks searched in the design doc's section 3.1
  order (user-looks, installed-looks, home-manager, system, shipped;
  `installed-looks` is empty until `look-install`, but the search includes
  it from the start) and rendered looks kept apart; `apply` renders into
  `$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/` when no rendered entry
  exists.
- The effectful checks the pure crate leaves to the tool: fonts through
  their carrier, the wallpaper through `scootbg check-image` (new: byte and
  pixel caps from the header before decoding), `scootbar daemon --check`.
- The transaction of section 4.5 under one `flock` (`<state>/look.lock`):
  render and validate, write `look.json` then the link (repairing the link
  from `look.json` whenever they disagree), scoped reloads in order (scoot,
  waiting for the wallpaper; every scootbar of the session), skip what is not
  running, roll back on a refusal, report "rolled back with errors" when a
  rollback reload also refuses, and list shadowed keys.
- The render cache pruned under the lock (active, previous, four most
  recent; never a directory `look.json` or the link names).
- An unwritable `<state>` refuses loudly.
- **The one-key `color-scheme` write** (D11, R2): `apply` writes
  `org.gnome.desktop.interface color-scheme` from the target look's
  `polarity`, under D11's rules (skip a key the Nix list claims or someone
  else changed; adopt it when there is no record yet), and records it in
  `look.json`. Without it a live swap across polarity leaves dark/light
  stale until the next activation, because `look-toolkit-live`'s full
  dconf set lands later. A box without `dconf` gets a note, not a refusal.
  **Adoption overwrites a user's own choice on non-Nix (F3):** with no record
  and no Nix list, the first `apply` replaces a `color-scheme` the user set
  in a settings app. Ship the opt-out with it: `--no-toolkit` per apply,
  and `scoot look toolkit off` (persisted in `look.json`) to skip every
  dconf write; document both beside the first-apply description. Before
  `look-nix-catalog`, home-manager restores its own `color-scheme` at the
  next activation; the dogfood settings below stop that.
- **Dogfood note in the docs:** on a home-manager machine before
  `look-nix-catalog`, set `programs.scoot.desktop.look = null`,
  `programs.scoot.stylix.enable = false` and `programs.scootbar.stylix.enable
  = false`, or the Nix-written values in the user's own files shadow every
  look; `scoot look current` lists anything still shadowing. Say what that
  costs: with `desktop.look = null` the profile's theme slot is inert too
  (`themeOn = theme.enable && look != null`, `nix/modules/theme-home.nix:37`),
  so GTK, Qt, fontconfig and foot lose their theming while dogfooding,
  until `look-nix-catalog`.
- Docs in the same PR: a new site page for looks (pick, apply, what is live
  and what is next-launch), and the generated CLI pages.

## Evidence to record

Tests: atomicity (a reader racing a swap sees the old or the new directory),
refuse-and-keep, rollback and rollback-with-errors, the lock (two applies
serialize), the link repaired from `look.json` after a simulated crash, a
missing bar skipped, a shadowed key reported, a pending user edit not applied
by a swap. Benchmarks on headless and `--tty`: swap wall time, frame times
across a swap, zero new per-frame allocations after it. The smoke test
applies two looks and checks the background pixel of each.

**Every test and the smoke run set their own `XDG_STATE_HOME`,
`XDG_RUNTIME_DIR` and `XDG_CACHE_HOME`** (R3): until `look-session-state`
lands, a headless session shares the user's state, so a test run on a dev
box without private dirs would repoint the developer's real look. A
`color-scheme` test also runs against a private dconf profile (or a fake
`dconf` on `PATH`), never the developer's session bus.

## Not in this ticket

`next`/`prev`/`random`/`--for` (`look-cycle-preview`), IPC, Nix,
third-party apps, authoring.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (2026-10-15)

PR #628: `scoot look list|current|show|validate|build|apply`
lands as a subcommand of the existing CLI (`scootctl::look`, parsed and run
through the client library like `msg`, so `scoot --help` and the JSON help
document cannot drift from it), plus `scoot look toolkit [on|off]` and
`scootbg check-image`. The catalog searches user, installed, home-manager,
system and shipped sources in order with origin by directory; `apply` runs
the section 4.5 transaction under one `flock` (render into a fresh cache
dir and rename, validate pure and effectful, write `look.json` then the
link, scoped reloads compositor-then-bars, rollback with rollback-errors,
offline shadowed keys, prune active/previous/four-recent). The toolkit
step writes `color-scheme` from polarity under the D11 rules with
`--no-toolkit` per apply and a persisted `toolkit off`, recorded in
`look.json` beside the choice. The dogfood settings and their cost are on
the new [looks page](../../../site/src/content/docs/looks/index.md).

Third fix round (same PR): nine review findings, each reproduced with a
failing test against the real binary or a byte-exact wire fake before
fixing — `cargo fmt` clean, rollback re-running partially-applied and
sent-but-unanswered compositor reloads (a `Reloaded` reply with a
non-empty `refused` holds the applied fields; a 30 s timeout or a close
after the write may hold the new layer; only an explicit error reply, the
"predates" wall or a never-sent request skips the re-run), shipped looks
applying on a `--no-default-features` bar (module tokens the installed
bar lacks are dropped with a note, the way popup keys are), the stage dir
created 0700 with dead-pid stages swept at start, `validate .` inside a
symlinked look dir preferring `$PWD`'s entry name, no `/tmp` litter from
the look tests, and one canonicalize in the catalog scan.

Evidence (dev VM, Linux, at the pushed head unless noted):

- `cargo build -p scoot -p scootbg -p scootbar`: clean (debug and
  release). Full-workspace clippy:
  `cargo clippy -p scoot -p scootctl -p scoot-look -p scootbg -p scootbar -p scoot-ipc --all-targets -- -D warnings`:
  clean; the `scootbar` per-feature matrix (default, none, each module
  alone, `popup` with each) clean.
- `cargo nextest run -p scootctl -p scoot-look -p scootbar -p scootbg
  --no-fail-fast`: 2380 run, 2380 passed, 7 skipped, 0 failed (incl. the
  new partial/unanswered rollback tests, the byte-exact `Reloaded` wire
  tests with the real field names, the timeout fake, and the minimal-bar
  drop tests), plus `cargo nextest run -p scoot --test look_apply_cli`:
  4 passed, 0 failed (the headless integration tests against the real
  binary: offline commands, two-look swap, pending-bind scope, JSON
  failure shapes).
- `cargo fmt --all --check`: clean. `scripts/backlog check`: 0 problems
  (run on macOS: the VM has no `python3`).
- `scripts/smoke-test.sh` with `SMOKE_REQUIRE_SCOOTBG=1`: exit 0, 45
  `ok:` lines (incl. the looks section: unknown-look refusal plus
  vinyl-sunset to ginger-night with pixel checks).
- Minimal bar, real binaries: `cargo build -p scootbar
  --no-default-features` beside `scoot`, then `scoot look build` and
  `scoot look apply` of a look carrying `[apps.scootbar]
  "workspaces.inactive-color"`: exit 0 with
  `bar without 'popup': rendered without popup-radius` and
  `bar without 'workspaces': rendered without workspaces tokens
  (inactive-color)`. The un-downgraded fragment against the minimal
  binary answers exactly
  `unknown field 'inactive-color', there are no fields` (and
  `unknown field 'popup-radius', expected one of ...` with the popup
  keys present); against the default binary the same fragment answers
  `ok` and no token is dropped.
- Live headless screenshots across a moonrise to ginger-night swap with
  differing background pixels (prior round, unchanged).

Swap cost, color-only case (dev VM, debug build, headless, 1 output, no
bar, dconf disabled, alternating `look-color-a` / `look-color-b`: two
color-only looks with color wallpapers `#2B3648` / `#3B4A5C`, no
wallpaper image, cache warm after 3 warmups; wall time per
`scoot look apply LOOK --state STATE --socket SOCK`, `scoot msg
screenshot` round trip per `--no-cursor --out shot.png`):

| run | apply wall ms | shot post-swap ms | shot at rest ms |
|---|---|---|---|
| 1–20 | 375, 373, 369, 376, 373, 394, 370, 372, 369, 368, 380, 366, 377, 369, 370, 373, 395, 373, 372, 370 | 73, 62, 59, 74, 55, 60, 67, 54, 60, 59, 66, 55, 74, 54, 56, 63, 54, 66, 57, 55 | 53, 51, 51, 51, 51, 51, 51, 51, 51, 51, 51, 51, 50, 50, 51, 50, 51, 51 |

Commands (with `SCOOT_LOOKS_DIR=/tmp/looks-628c`,
`XDG_*_HOME`/`HOME` under `/tmp/swapA-628c`, `DBUS_SESSION_BUS_ADDRESS=disabled:`,
session `scoot --headless --width 800 --height 600 --socket SOCK --config empty.toml`).

Swap cost, realistic case (dev VM, release build
`cargo build --release -p scoot -p scootbar -p scootbg`, headless, 1
output, live default bar on the session display, `scootbg` on `PATH`,
dconf disabled, alternating `look-img-a` / `look-img-b`: image
wallpapers from ginger-night / moonrise, cache warm after 3 warmups,
N=20, same commands with the release binary):

| run | apply wall ms | shot post-swap ms | shot at rest ms |
|---|---|---|---|
| 1–20 | 468, 424, 473, 421, 474, 422, 475, 424, 421, 423, 471, 423, 466, 420, 472, 425, 472, 420, 471, 422 | 5, 9, 31, 8, 29, 7, 44, 7, 29, 5, 29, 6, 32, 5, 15, 6, 30, 6, 32, 5 | 24, 24, 24, 41, 41, 42, 41, 25, 24, 41, 25, 24, 41, 41, 42, 42, 25, 25, 41, 26 |

Post-swap `scoot msg screenshot` round trips sit with at-rest ones in
both cases; per-frame allocation audit after a swap is still open (no
instrumentation landed for it).
