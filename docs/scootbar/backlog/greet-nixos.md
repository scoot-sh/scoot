---
title: "scootgreet: NixOS module, the static look, the supervisor and agreety fallback, opt-in"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-users-sessions, greet-power, look-format"
milestone: "M7"
---

# scootgreet: NixOS

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#stylix-and-look-integration), its
[failure modes](../../../dev/design/scootgreet.md#failure-modes-never-strand-a-login), and the
[theming contract](../../../dev/design/shell-apps.md#the-greeter-is-static) (live looks, PR #578: the
greeter is static). Serves **daily-drive**.

## What to do

- **`programs.scootgreet`** (NixOS), and `programs.scoot.greeter.daemon` = `"regreet" |
  "scootgreet"`, default `"regreet"`.
- **With scootgreet:**
  - greetd's command is `scootgreet session || scootgreet-fallback`, everything by absolute
    store path;
  - `fallback-session` is the scoot session's `Exec`;
  - tmpfiles create `/var/lib/scootgreet`;
  - `services.xserver.xkb.*` go into the greeter's environment;
  - `/etc/scoot/greet.toml` holds the user's `settings` only;
  - the look fragment is rendered at build time from `programs.scoot.greeter.look` by #578's
    renderer (`look-greeter`).

## Acceptance

`nix/scootgreet-tests.nix` pins:

- the rendered files;
- the command and its fallback;
- XKB env and tmpfiles;
- today's ReGreet output byte-identical with the default.

On the dev VM, with greetd 0.10.3:

- a real login;
- wrong-then-right;
- a SIGKILL mid-prompt landing on agreety;
- cage SIGKILLed, with agreety's prompt readable (the VT is back in text and keyboard mode);
- a broken look fragment drawing the defaults and still logging in;
- `Ctrl+Alt+F2` reaching a getty.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The transcripts and a screenshot of each state.

## Not in this ticket

The ISO.
