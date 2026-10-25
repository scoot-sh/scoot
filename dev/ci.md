# CI shape (why the splits exist)

`ci.yml` gates every PR by path (`changes` job); a docs-only push runs
nothing. The heavy jobs overlap instead of queueing behind one another:

- `linux` (default features: build, clippy, nextest, `cargo test`, smoke)
  is the compositor gate. It does no nix evaluation of its own.
- `nix-check` evaluates the flake once (`nix fmt`, `nix flake check`,
  `nix flake show --all-systems`) in parallel with `linux`. It replaced
  the serial `nix flake check` that measured 5-9 min inside `linux`. It
  covers the Linux systems only; `macos` keeps its own `nix flake check`
  for the Darwin ones.
- `linux-xwayland` and `linux-minimal` are the two halves of the old
  serial `linux-features` job (measured 24-36 min, the critical path).
  Each restores the `linux` cargo cache read-only and rebuilds Smithay
  under its own unification, so the run costs the max of the two, not
  the sum. Neither saves: a feature-rebuilt `target/` is unusable to the
  default build and would only buy a second multi-GB cache line.
- `macos` is client-only (`cargo check`, `scoot msg --help`) plus the
  Darwin `nix flake check`, which a Linux run cannot do.
- `scootbar-modules`, `scootbar-lint`, `scootbar-test` and `scootbar-fuzz`
  are the four quarters of the old serial `scootbar` job (measured
  ~21.5m in run 40230223919, the full-run critical path after the split
  above: module check ~5m, clippy ~3m, nextest ~7m, release ~1.3m,
  fuzz ~2.2m, plus restore). Each cargo quarter restores the shared
  `scootbar-cargo-` line read-only and no job relies on a sibling's
  `target/` dir, so the run costs the max of the four, not the sum. Only
  `scootbar-test` saves on main, so the repo keeps a single
  `scootbar-cargo-` line under the 10 GB budget. No test, matrix entry,
  fuzz run or check removed; REQUIRE-style envs kept.
- `scootbg`, `scootlaunch`, `scootnotify`, `portal`, `look`
  keep their own cargo caches (no Smithay inside) and run only when
  their paths (or shared deps) change; the `-integration` jobs restore
  the `linux` cache read-only and add the live-compositor runs.
- The `changes` job also runs its own filter tests
  (`scripts/test_check_ci_filters.py` plus `scripts/check-ci-filters.py`,
  derived from `cargo metadata`): a shared-crate change that skips a
  dependent fails the run, so the filters cannot rot silently. Fail open
  (unknown path runs everything, including `portal`) stays.

`concurrency.cancel-in-progress` applies to PR pushes only, never main.
Cache saves happen on main only; PR runs restore and save nothing.
`nix-build.yml` still owns the release builds, not per-PR CI.
