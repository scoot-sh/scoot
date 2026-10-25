# CI shape (why the splits exist)

`ci.yml` gates every PR by path (`changes` job); a docs-only push runs
nothing. The heavy jobs overlap instead of queueing behind one another:

- `linux` (default features: build, clippy, nextest, `cargo test`, smoke)
  is the compositor gate. It does no nix evaluation of its own.
- `nix-check` evaluates the flake once (`nix fmt`, `nix flake check`,
  `nix flake show --all-systems`) in parallel with `linux`. It replaced
  the serial `nix flake check` that measured 5-9 min inside `linux` plus a
  duplicate Darwin eval inside `macos`; same `scoot` gate covers both.
- `linux-xwayland` and `linux-minimal` are the two halves of the old
  serial `linux-features` job (measured 24-36 min, the critical path).
  Each restores the `linux` cargo cache read-only and rebuilds Smithay
  under its own unification, so the run costs the max of the two, not
  the sum. Neither saves: a feature-rebuilt `target/` is unusable to the
  default build and would only buy a second multi-GB cache line.
- `macos` is client-only (`cargo check`, `scoot msg --help`); its flake
  eval lives in `nix-check`.
- `scootbg`, `scootbar`, `scootlaunch`, `scootnotify`, `portal`, `look`
  keep their own cargo caches (no Smithay inside) and run only when
  their paths (or shared deps) change; the `-integration` jobs restore
  the `linux` cache read-only and add the live-compositor runs.

`concurrency.cancel-in-progress` applies to PR pushes only, never main.
Cache saves happen on main only; PR runs restore and save nothing.
`nix-build.yml` still owns the release builds, not per-PR CI.
