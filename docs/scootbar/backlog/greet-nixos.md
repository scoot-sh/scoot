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
  - greetd's `default_session.command` is the **`scootgreet-greeter` wrapper script's absolute
    store path**, nothing more. greetd builds `exec <command>` and runs it with `/bin/sh -c`
    (`greetd/src/session/worker.rs:228-235,267-272`), so a `||` in the command text never runs
    ([why](../../../dev/design/scootgreet.md#failure-modes-never-strand-a-login)). The wrapper's body is
    `scootgreet session || exec scootgreet-fallback`, by absolute paths;
  - the startup deadline comes from `greet-baseline`'s measured first frame (option
    `startupTimeout`, default 15 s);
  - `fallback-session` is the scoot session's `Exec`;
  - tmpfiles create `/var/lib/scootgreet`;
  - `services.xserver.xkb.*` go into the greeter's environment;
  - `/etc/scoot/greet.toml` holds the user's `settings` only;
  - the look fragment is rendered at build time from `programs.scoot.greeter.look` by #578's
    renderer (`look-greeter`).

## Acceptance

`nix/scootgreet-tests.nix` pins:

- the rendered files;
- `default_session.command` equals the wrapper's store path exactly, and contains no `||`, `;`
  or `&&`. The broken `cmd || fallback` form fails eval, so the tests cannot lock it in;
- the wrapper script's body: the supervisor `||` an `exec` of the fallback, all absolute paths;
- the fallback's body: agreety re-run only on a non-zero exit, with the session command
  substituted at build time, **non-empty and absolute** (an eval pin), never read from a
  variable or `greet.toml`;
- the wrapper and the fallback are **executable** (mode `0755`). A non-executable wrapper exits
  126 on every start and loops greetd into its start limit;
- the wrapper logs the supervisor's exit status itself, and a failed log write cannot block the
  fallback;
- XKB env and tmpfiles;
- today's ReGreet output byte-identical with the default.

On the dev VM, with greetd 0.10.3:

- a real login;
- wrong-then-right;
- a SIGKILL mid-prompt landing on agreety;
- **the supervisor itself failing, through the real wrapper as greetd runs it**: the
  `scootgreet` binary missing, `scootgreet session` exiting non-zero at once, and
  `scootgreet session` aborting. Each must reach a working agreety login **that starts the
  scoot session**, and the wrapper's own line in `/var/lib/scootgreet/fallback.log` must record
  the exit status;
- the same with `/var/lib/scootgreet` read-only: the fallback still runs;
- agreety's `--max-failures` used up, ending in a fresh greeter, not a dead VT;
- suspend and resume at the login screen, with no fallback (monotonic timers);
- cage SIGKILLed, with agreety's prompt readable (the VT is back in text and keyboard mode);
- a broken look fragment drawing the defaults and still logging in;
- `Ctrl+Alt+F2` reaching a getty.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The transcripts and a screenshot of each state.

## Not in this ticket

The ISO.
