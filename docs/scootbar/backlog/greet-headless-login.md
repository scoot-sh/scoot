---
title: "A headless greetd login for agents and VM tests (`scootgreet login`), no UI"
status: "open"
area: "scootbar"
priority: "low"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# A headless greetd login (`scootgreet login`)

Filed 2026-10-12 from the #579 re-review. Serves **computer use**. It is a candidate early
ticket: anything running as the greeter user with `GREETD_SOCK` can drive greetd, so an agent
or VM test logging in does not need the graphical greeter. Today this is done with an ad-hoc
script.

## What to do

Create `crates/scootgreet`, with its CI classify output and job and a CLAUDE.md scope entry.
For now it holds only:

- **the frame codec**: a native-endian u32 plus JSON, capped at 64 KiB;
- **the pure conversation state machine** with the
  [cancel discipline](../../../dev/design/scootgreet.md#the-greetd-client): `cancel_session` before every
  `create_session`, after every error, and on reconnect, because greetd 0.10.3 keeps a failed
  or abandoned session;
- **`scootgreet login --user U [--session NAME]`**, which answers every prompt from stdin or
  `--answers-fd`, never from argv;
- **a fake greetd that models greetd's retained session state**, checked against the real one.

There is no Wayland, no drawing and no keyboard. `greet-greetd-client` adds the supervisor and
the screen on top of this crate.

## Acceptance

On a real greetd 0.10.3 on the dev VM:

- a login into a scoot session;
- a wrong answer and then the right one in the same run;
- an OTP second prompt.

Also: a fuzz target on frames, and a test that no answer appears in `/proc/PID/cmdline`.

## Evidence the review needs

- The standard set re-run by the reviewer: `cargo nextest run -p scootgreet`, clippy, fmt and
  the fuzz target.
- The real-greetd transcripts (journal lines, no secrets), with the SHA.

## Not in this ticket

The graphical greeter, the supervisor, the locked page.
