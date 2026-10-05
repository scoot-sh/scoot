---
title: "Remove the standalone scootctl client; scoot msg is the only client"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Remove the standalone scootctl client; scoot msg is the only client

## Resolution (PR #464, 2026-10-05)

Landed as `feat(scoot,scootctl,nix)!` on branch `feat/scootctl-with-scoot`,
rebased onto `origin/main` 7af5a3319. Deleted `crates/scootctl/src/main.rs`
(the crate stays as the client library; name kept, so the commit scope stays
valid), the flake package/app/overlay entries (Darwin default is now `scoot`,
client-only there), the macOS-standalone CI shape (macOS job checks `scoot`
and runs `scoot msg --help`), and the smoke-test equivalence section. Help,
JSON (`binary: scoot`, client `scoot msg`), docs, comments, scripts and bench
harnesses name `scoot msg`; site `scootctl/` → `msg/` with redirects. No
compat link. Negative pins: `!(overlaid ? scootctl)` (Linux + Darwin) in
`nix/tests.nix`, green via `nix flake check` on aarch64-linux.

Evidence: Asahi M2 nextest `-p scoot -p scootctl -p scoot-ipc -p scoot-core`
2508 passed; `cargo test -p scoot` 2124 passed; workspace clippy/fmt clean;
headless smoke 36 oks; four nix package builds green with no `bin/scootctl`;
docs-site build green; Mac-built `scoot msg` drove Asahi headless over a
forwarded socket (version/outputs/windows). Full workspace nextest: only
pre-existing environmental scootbar failures (no sound server). Downstream
read-only sweep: nixos-webtop already `scoot msg`-only, scoot-iso and
`~/nixos-config` have no client refs — nothing to follow up.

Filed 2026-10-05. Serves **daily-drive**: one client name to learn, install and script (`scoot msg`), instead of two that must be kept identical. (Maintainer FINAL, 2026-10-05: no compatibility link either — "I'm the only user.")

## The gap

Two client entry points exist for one socket: the standalone `scootctl` binary (`crates/scootctl/src/main.rs`, flake `packages.scootctl`, `apps.scootctl`, the macOS default and CI job) and `scoot msg` (the alias in `crates/scoot/src/cli.rs`, parsed through the same `scootctl` library crate). Every help table, error string and exit code must be kept identical across both by containment tests and a smoke-test equivalence section (`scripts/smoke-test.sh`, "scoot msg vs scootctl") — process for zero user benefit. The docs name `scootctl` in 100+ places (`site/src/content/docs/`, `README.md`) that all have to say `scoot msg` instead.

## What to do

Delete the `scootctl` binary target (keep the crate as the client library behind `scoot msg` — the name stays, so `scootctl` remains a valid commit scope; renaming would churn every `use scootctl::` for no user-visible gain). Drop its flake package/app/overlay entries (Darwin default becomes `scoot`, client-only there), its CI shape (macOS job builds/tests `scoot`), and the smoke-test equivalence section. Rename help/JSON/docs to `scoot msg` (`binary` is `scoot`, client usage names `scoot msg`); move the site `scootctl/` section to `msg/` with redirects. No `bin/scootctl` link, no argv[0] behavior, no migration note beyond the `BREAKING CHANGE:` footer. Breaking: PR title `feat(scoot,scootctl,nix)!:`.

## Not in this ticket

History stays verbatim (backlogs, roadmap, CHANGELOG, benchmarks, `Asahi.md`). Downstream (`yackey-labs/nixos-webtop`, `scoot-sh/scoot-iso`, `~/nixos-config`) is read-only here; the orchestrator updates it after merge.
