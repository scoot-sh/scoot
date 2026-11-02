---
title: "Harden look-target reloads and paths: bounded pipes, NFC duplicates, write-site path check"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-02"
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

## Resolution

Landed 2026-11-02 in PR #874 (`fix(scoot-look,scootctl): bound reload
runs; ASCII-only outputs; join-site guard`).

1. Reload timeout bounds the call: new shared helper
   `crates/scootctl/src/look/bounded.rs` serves the target reload runner
   and the pre-existing `run_with_timeout` (one helper, not two copies).
   The child leads its own process group, both pipes drain on
   nonblocking reads, and past the deadline the group is killed
   (`rustix::process::kill_process_group`, no `unsafe`) and the child
   reaped — nothing waits for pipe EOF. The review input
   (`sleep 8 & exec sleep 0`, 2 s) returns in 2.1 s (was 8.1 s), and a
   `ps` scan for a uniquely-named wrapper grandchild proves the group
   dies with it.
2. Output capture bounded: 64 KiB per stream (`MAX_OUTPUT_BYTES`, the
   usual pipe capacity), then the group is killed and the note says the
   output was truncated — a 300 s `yes` holds 128 KiB, never gigabytes.
   (Pre-fix the chatty probe additionally deadlocked to the full 30 s
   default: nobody drained until exit, so the pipe buffer filled.)
3. Unicode duplicates: no normalization crate sits in the lock tree
   (`unicode-normalization` count 0; only `unicode-ident`, no ICU/idna),
   so adding one would be a new supply-chain dependency — the smaller
   change is refusing non-ASCII outputs at parse. Both NFC and NFD
   spellings refuse, so neither stages; templates keep Unicode names.
   The duplicate check stays case-insensitive over ASCII-only outputs.
4. Write-site re-check: `joinable_output` (relative, no `..`) guards
   `write_render_dir` (`InvalidInput`) and `cached_complete` (never
   complete), pinned with hand-built targets.
5. Docs: the look-targets page says the duplicate refusal is
   case-insensitive and documents the ASCII rule and the 64 KiB cap.
6. Review non-blocking 7 needs no action, agreed: install, preview and
   validate thread the same `config: Option<&Path>` through
   `load_target_files`, with `None` resolving the live environment — the
   fail-fast coupling is the design, same-user only.

Evidence per input (dev VM, before → after): fork-timeout 8.104 s →
2.105 s; group-kill residue scan 8.109 s → 2.104 s clean; chatty
30.132 s → 0.158 s with truncation note; helper cap 30.029 s →
0.108 s naming 65536; NFC/NFD shape FAIL (`[]`) → refuse;
write-site writes/counts-complete → refuses/false. Gates:
`cargo fmt --all --check`, `cargo clippy -p scoot-look -p scootctl
--all-targets -- -D warnings`, `cargo nextest run -p scoot-look
-p scootctl --no-fail-fast` (664 passed), `cargo test -p scoot-look
-p scootctl` (all suites ok), `python3 scripts/backlog check`
(0 problems). No new crate in the tree (`Cargo.lock` untouched).
