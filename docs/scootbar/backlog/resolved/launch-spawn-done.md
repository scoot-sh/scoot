---
title: "scootlaunch: launching right (activation token, environment, systemd scope, fallback fork)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootlaunch: launching

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#launching-correctly); split in the #579 fix
round. Serves **daily-drive**.

## What to do

- **The activation token**, from the choosing key or button serial; wait for `done` with a
  200 ms bound.
- **The environment rules**: strip the launcher's own token, set the new one, `Path=`, stdin
  from `/dev/null`.
- **The systemd scope**: `StartTransientUnit` into `app-scootlaunch-<id>-<random>.scope`,
  bounded at 500 ms. Confirm which slice systemd's desktop-environment page asks for.
- **The fallback**: a double fork with `setsid`.
- **`DBusActivatable` entries**, through `org.freedesktop.Application` with `activation-token`
  in `platform_data`.

## Acceptance

- A mouse-picked entry re-activates an already-running single-instance stub (headless).
- The launched app shows in its own scope under a user manager.
- The fallback is exercised with no bus.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/panel.md` with what a pick launches (scope, environment, token, fallback) and `DBusActivatable`; troubleshooting `> **Symptom:**` for an app that starts outside its scope. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `systemctl --user status` output.

## Not in this ticket

dmenu mode.

## Resolution (2026-10-19, PR #676)

A picked entry launches instead of printing its id. `crates/scootlaunch/src/spawn.rs`
(token from the choosing serial with a 200 ms bound, the environment rules, `Path=`
newly indexed with the cache at v2, `StartTransientUnit` into
`app-scootlaunch-<id>-<random>.scope` under `app.slice` with a 500 ms bound, the
detached double-fork fallback with no bus, `Activate`/`ActivateAction` with
`activation-token` in `platform_data`); the panel mints the token, launches,
draws spawn failures for 2 s, and records frecency. Exit codes unchanged
(0/1/2); stdout on a pick is now empty. Slice confirmed as `app.slice` from
systemd's desktop-environment page. Docs in the same PR (`panel.md`, `cli.md`).
Evidence: 229 nextest + opposite-runner green, six mutation pins red-then-green,
release 1,250,032 bytes linking only libxkbcommon past std, and
`systemctl --user status` showing a launch in its scope under `app.slice`.
