---
title: "scootgreet: NixOS module, the static look, the agreety fallback wrapper, opt-in"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-ui"
milestone: "M7"
---

# scootgreet: NixOS

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#stylix-and-look-integration),
its [failure modes](../../../dev/design/scootgreet.md#failure-modes-never-strand-a-login), and the
[theming contract](../../../dev/design/shell-apps.md#the-greeter-is-static) (live looks, PR #578: the
greeter is static). Serves **daily-drive**.

## What to do

- **`programs.scootgreet`** (NixOS) and **`programs.scoot.greeter.daemon`** =
  `"regreet" | "scootgreet"`, default `"regreet"`.
- **With scootgreet:**
  - greetd's command is `scootgreet-session`: cage without `dbus-run-session`, and on a
    non-zero exit, `exec agreety --cmd <default session>`;
  - tmpfiles create `/var/lib/scootgreet`;
  - `services.xserver.xkb.*` go into the greeter's environment;
  - `/etc/scoot/greet.toml` holds the user's `settings` only;
  - `/etc/scoot/look/greet.toml` is rendered at build time from `programs.scoot.greeter.look`
    by the contract's renderer (#578's `look-greeter` ticket; if it has not landed, this
    ticket renders the fragment through `scoot-look` itself).
- **The loader** refuses non-allowlisted keys, applies the grammar, and falls back to
  built-in defaults on any fragment error.

## Acceptance

- `nix/scootgreet-tests.nix` pins:
  - the rendered files;
  - the wrapper and its fallback;
  - XKB env and tmpfiles;
  - today's ReGreet output byte-identical with the default.
- On the dev VM:
  - a real login;
  - an injected crash lands on agreety, never a dead VT1;
  - a broken look fragment draws the defaults and still logs in;
  - `Ctrl+Alt+F2` reaches a getty.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The crash-fallback transcript and a screenshot of each state.

## Not in this ticket

The scoot-hosted mode, the ISO.
