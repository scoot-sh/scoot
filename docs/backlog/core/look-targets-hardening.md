---
title: "Harden look-target reloads and paths: bounded pipes, NFC duplicates, write-site path check"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Harden look-target reloads and paths: bounded pipes, NFC duplicates, write-site path check

Filed 2026-11-02 from the security review of PR #871 (look-user-targets
slice 1). Serves **daily-drive**: a reload that never returns freezes
`scoot look apply`, `sync` and the watch-save path, and a compositor-adjacent
tool must not hang on a user's own config.

## The gap

1. **The reload timeout does not bound a reload that forks and keeps the
   pipes open.** `argv = ["/bin/sh", "-c", "sleep 8 & exec sleep 0"]` with
   `timeout_secs = 2` blocks `run_reloads` for 8.0 s; a daemon-style reload
   that never closes stdout hangs apply, sync and watch-save with the swap
   already standing. Cause: `try_wait` then `wait_with_output` waits for
   pipe EOF, past the deadline. The older `run_with_timeout` in scootctl has
   the same shape, so it is inherited, not new.
2. **Reload stdout and stderr are captured without a bound.** Only the
   displayed excerpt is truncated. A `yes`-style reload at the 300 s maximum
   timeout captures gigabytes.
3. **Unicode duplicates both stage.** Outputs `caf\u00e9/x.conf` (NFC) and
   `cafe\u0301/x.conf` (NFD) pass the duplicate check, which compares bytes
   case-insensitively, and the second wins silently. APFS treats them as one
   file.
4. **No write-site re-check.** `write_render_dir` and `cached_complete` trust
   a validated path; an absolute `output` in a `join` discards the prefix.
   Unreachable through `discover` today.
5. Docs: the duplicate refusal is case-insensitive in code; the look-targets
   page does not say so.

## What to do

Drain the pipes with a byte cap and never wait past the deadline (kill the
process group, not just the child), in the target reload runner and in
`run_with_timeout`. Normalize (NFC) before the duplicate comparison. Reject
absolute and `..` outputs at the join as defense in depth. Say "case-insensitive"
on the docs page. Pin each with a test using the inputs above.

## Not in this ticket

Following intermediate symlink components in a template subpath (same-user
reads of files the user can already read; noted only).
