---
title: "Gate integration tests on their module features, and tidy scootbar-unit-test.sh"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
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

## Resolution (2026-10-08, PR #527)

Landed as the ticket asks, all three gaps, evidence keyed to the Asahi M2
(aarch64-linux; commits `6f8206989`/`798f5dca8` on `chore/scootbar-test-gating`):

1. Every module-placing integration test exists only where its module does:
   file-level `#![cfg]` where the file is uniform (`clock`, `pointer`,
   `icons`, `msg`, `hermetic` on `clock`; `popup` on `volume`+`popup`;
   `window_title` on `window-title`; `outputs` on `clock` with per-test
   `workspaces`), per-test `#[cfg]` where mixed (`workspaces`, `agent`,
   `exec`, `tooltip` plus its `volume` test, one `hotplug` test), helpers
   and imports gated with their users (the `appearance.rs` pattern).
   With a scoot beside the binary: `--no-default-features` 484 passed
   (was 21 failed), `--features clock` 589 passed (was 9 failed),
   `--features workspaces` 570 passed (was 16 failed), each under both
   `cargo nextest run` and `cargo test` with `SCOOTBAR_REQUIRE_SCOOT=1`
   `SCOOTBAR_REQUIRE_SWAY=1` `SCOOTBAR_REQUIRE_DBUS_DAEMON=1`. The
   `scootbar-integration` CI job runs the same tests against all three
   reduced-feature bars. `docs/scootbar/testing.md` states the rule.
2. `scripts/scootbar-unit-test.sh` `cleanup()` removes the bar's control
   socket and lock on the run's own display (`scootbar-$WL.{sock,lock}`,
   the run's own headless scoot's) and reports the leftover count; the
   header's stale-socket paragraph now says so.
3. S8 in `--nixos` mode copies the module's own generated
   `graphical-session.target.wants/scootbar.service` link (and dies naming
   the units dir if it ever goes missing) instead of hand-making one;
   `--home` S8 is unchanged, its generated link is exercised in S9.

Not run: the script's full S0-S9 on hardware. The M2 holds a live
`WAYLAND_DISPLAY` in the user manager plus the maintainer's own installed
`scootbar.service`, so the script refuses in both modes (exit 2) exactly as
documented. In place of the run: `bash -n` clean; the S8 link block run
verbatim against the real nix-evaluated units (link `-> ../scootbar.service`
present and copied); the cleanup block run in isolation (removes only the
run's pair, no-op with `WL` empty under `set -u`); the release binary is
bit-for-bit unaffected (the diff touches no binary input). Needs a
maintainer hardware run to close that loop.

## Not in this ticket

The NixOS switch, a lingering user manager and a real x86_64 user manager;
those are listed as unverified in the resolved
[nix-modules-and-stylix](resolved/nix-modules-and-stylix-done.md).
