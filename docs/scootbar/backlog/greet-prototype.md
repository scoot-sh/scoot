---
title: "scootgreet go/no-go prototype, out of the workspace: real greetd, the cancel discipline, typing"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "greet-baseline; maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# scootgreet prototype: the go/no-go gate

Filed 2026-10-12 in the #579 fix round. The gate moved out of `greet-ui` to run before any
extraction, and it tests what can actually fail. Serves **daily-drive**.

## What to do

A throwaway binary **outside the workspace**, `#[path]`-including scootbar's drawing and text
sources and the keyboard spike's pick. It runs a full-screen password prompt under cage and
the [greetd client](../../../dev/design/scootgreet.md#the-greetd-client) with the cancel discipline:

- `cancel_session` before every `create_session`;
- `cancel_session` after every error;
- `cancel_session` on reconnect.

Plus the supervisor's fallback to agreety.

## Acceptance

Against a **real greetd 0.10.3** on the dev VM:

- a wrong password, then the right one, in the same greeter, logs in (greetd keeps a failed
  session until cancelled: `server.rs:73`, `session/interface.rs:159-165`, `context.rs:185-186`);
- the prototype SIGKILLed with a secret prompt pending lands on a working agreety login;
- the PAM matrix: password, expired-password change, an OTP second prompt, info-only;
- every layout the keyboard spike passes, typed into the password field, Compose included;
- RSS and first frame against `greet-baseline`'s ReGreet numbers;
- **a weight case that is still real**: `greet-baseline`'s marginal closure over the installed
  desktop profile is at least about 300 MiB, or the scoot-iso image is still over its size cap
  with ReGreet on it. Otherwise this is a no, whatever the other rows say.

A "do not build" criterion from the design ends the epic.

## Resolving this gate

The tickets it guards name it in prose ("GO verdict recorded in greet-prototype"), which `scripts/backlog resolve` does not strip ([why](../../../dev/design/shell-apps.md#how-the-gates-stay-closed)). Record the verdict here, then resolve. On **go**, a human deletes that clause from the guarded tickets. On **no**, a human resolves the epic and the guarded tickets as won't-do, citing this evidence.

## Evidence the review needs

The greetd and agreety journal for each case (no secrets), the rows with the SHA, and the
prototype's source (deleted after).
