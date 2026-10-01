---
title: "Gate integration tests on their module features, and tidy scootbar-unit-test.sh"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# Gate integration tests on their module features, and tidy scootbar-unit-test.sh

Filed 2026-10-01, from the review of the Nix modules PR (#370). Serves
**daily-drive**: a build with fewer modules should have a test suite that
tests what it builds, and the test script should leave a user's session as it
found it.

## The gap

1. **Integration tests assume every module is built.** With a scoot beside the
   test binary, `cargo nextest run -p scootbar --no-default-features` fails 21
   of 325 tests, `--features clock` 9 of 393 and `--features workspaces` 16 of
   377 (`'left': no module 'clock' in this build (it has: none)`). The same 21
   fail on `main`, so none of it came from #370. CI and the per-feature unit
   matrix run `--bin scootbar`, which does not reach them.
2. **`scripts/scootbar-unit-test.sh` leaves a stale control socket** in the
   user's runtime dir (`$XDG_RUNTIME_DIR/scootbar-wayland-N.sock`, plus its
   lock): S2 and S3 SIGKILL the bar, the header documents it, and the
   header's "removes what it made" is true only apart from that. The display
   `N` is one the script's own headless scoot just created, so `cleanup()`
   could remove `scootbar-$WL.{sock,lock}` after killing that scoot.
3. **S8 in `--nixos` mode does not test the generated `WantedBy` link**:
   `scripts/scootbar-unit-test.sh` makes the wants link by hand, so the
   check proves systemd's wants semantics, not the module's output. `--home`
   reads the generated link (`gendir` in S9). Link the generated one, or
   assert it exists.

## What to do

Gate the module-specific integration tests on their features (or skip with a
stated reason) and run them in the per-feature matrix; fix 2 and 3 in the
script and run it on the Asahi box as documented (it refuses on a live
session, so it is safe there).

## Not in this ticket

The NixOS switch, a lingering user manager and a real x86_64 user manager;
those are listed as unverified in the resolved
[nix-modules-and-stylix](resolved/nix-modules-and-stylix-done.md).
