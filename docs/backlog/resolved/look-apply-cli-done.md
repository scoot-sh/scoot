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

PR https://github.com/scoot-sh/scoot/pull/628 (open, not merged): `scoot look list|current|show|validate|build|apply`
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

Second fix round (pushed as `53bff9e73`, same PR): ten review findings,
each reproduced with a failing test against the real binary or a byte-exact
wire fake before fixing — the scoped bar reload as a distinct `reload-look`
verb (a real pre-scope bar applied a pending `height 28 -> 90` on the old
bytes and answers `unknown request` with nothing applied on the new ones),
rollback only re-running positive receipts, whole-stderr bar checks with a
popup-less re-render (real `--no-default-features` binary beside `scoot`:
`apply` and `build` exit 0), conditional `toolkit off` with tri-state dconf
reads, `validate .`, the bar check in `validate`, an honest first-apply
note, followed symlinks for user configs and farm manifests, and guarded
`build --out` staging. Details and per-finding evidence:
`scratchpad/fix628b-report.md` (unpublished working notes, not in this PR).

Evidence (dev VM, Linux, both at `53bff9e73` unless noted):

- `cargo build --workspace`: clean.
- `cargo nextest run --workspace --no-fail-fast`: 5479 run, 5477 passed,
  50 skipped, 2 failed — the pre-existing xwayland `refused_cost` fd-budget
  failure (fails on main too) and one new test in pre-hardened form (fixed
  the same day; the rerun over `scootctl`/`scoot-look`/`scootbar`/`scoot`
  is 4456 run, 4455 passed, 46 skipped, 1 failed: only the xwayland one).
- `cargo test -p scootctl -p scoot-look -p scootbg -p scootbar`: every suite
  ok, 0 failed (incl. 80 `scootctl` look unit tests and the 4
  `look_apply_cli` headless integration tests against the real binary:
  offline commands, two-look swap, pending-bind scope, JSON failure shapes).
- `cargo clippy -p scoot -p scootctl -p scoot-look -p scootbg -p scootbar -p scoot-ipc --all-targets -D warnings`: clean; the `scootbar`
  per-feature matrix (default, none, each module alone, `popup` with each)
  clean; `scootbg`/`scootbg-mem` clippy clean with no `libc` dependency.
- `cargo fmt --check`: clean. `scripts/backlog check`: 0 problems.
- `scripts/smoke-test.sh` with `SMOKE_REQUIRE_SCOOTBG=1`: passes.
- `nix build .#docs-site`: passes (llms, page-link, asset, snippet and
  mdx-twin gates with the new pages).
- Live headless screenshots across a moonrise to ginger-night swap with
  differing background pixels (prior round, unchanged).

Swap cost (dev VM, `53bff9e73` debug build, headless, 1 output, no bar,
dconf disabled, alternating two color-only looks, cache warm after 3
warmups; wall time per `scoot look apply`, frame proxy per `scoot msg
screenshot` round trip):

| run | apply wall ms | shot at rest ms | shot post-swap ms |
|---|---|---|---|
| 1–20 | 372, 369, 369, 373, 371, 375, 370, 373, 371, 378, 374, 374, 377, 373, 376, 370, 375, 373, 375, 374 | 219, 189, 179, 185, 186, 184, 185, 184, 172, 177, 180, 174, 181, 173, 173, 176, 181, 179, 185, 185 | 176, 200, 182, 183, 178, 188, 179, 179, 184, 187, 188, 188, 186, 187, 191, 183, 188, 184, 185, 188 |

Supplement: screenshots taken concurrently *during* six further swaps
(36 samples): 169–179 ms, indistinguishable from rest. The swap adds no
frame debt on this path; per-frame allocation audit after a swap is still
open (no instrumentation landed for it).
