---
title: "Skip every udiskie keyring test, not two"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Skip every udiskie keyring test, not two

Filed 2026-10-24. Serves **daily-drive**: anyone building the ISO or a
system that carries scoot's default `udiskie` on x86_64 hits it, and it
costs a failed build, not a feature.

## The gap

v0.6.2 ships `udiskie` with `test/test_keyutils.py` and `test_is_valid`
disabled (#729), because the suite needs a working kernel keyring. That
covers the sandbox flake but not Docker's default seccomp profile, which
refuses `keyctl(2)` with EPERM. Under that profile the five
`test_cache.py::TestPasswordCache` tests (`test_clear`, `test_invalidate`,
`test_timeout`, `test_touch`, `test_update`) still fail
(scoot-iso run 40115451158, `docker-build` job log).

So `scripts/build-iso-docker.sh` fails for anyone running it locally with
Docker's default profile. scoot-iso's CI sets `ISO_DOCKER_SECCOMP_UNCONFINED=1`
for its `docker-build` job (opt-in, off by default) to get past it. A local
user has no such help.

## What to do

Widen the override that #729 added to the one shared udiskie definition so
it skips every test that touches the kernel keyring: all of
`test/test_cache.py::TestPasswordCache` as well, or the whole of
`test_cache.py`. Check the exact list against the pinned nixpkgs by
running the check phase under Docker's default seccomp profile, not only
natively. Then drop `ISO_DOCKER_SECCOMP_UNCONFINED` from scoot-iso's CI
and `scripts/build-iso-docker.sh`.

The review of #729 ran the tests natively on aarch64 and did not catch
this. The verification that decides this ticket runs in a Docker container
with the default profile.

## Not in this ticket

Reporting the flaky tests to nixpkgs (the maintainer decides that).
