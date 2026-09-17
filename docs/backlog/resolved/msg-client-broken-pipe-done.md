---
title: "`flexwm msg` panics when its stdout reader goes away (EPIPE) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flexwm msg` panics when its stdout reader goes away (EPIPE) — RESOLVED

Found 2026-09-17, reviewing PR #61
(`resolved/screencopy-shell-thumbnails-fallback-done.md`), filed rather
than fixed inside it. **RESOLVED 2026-09-17** (PR #62): the diagnosis below
was exactly right, and the fix covers every client-binary stdio site, not
just the one line.

## The gap

`msg.rs:40` prints the reply with `println!`, which panics on EPIPE —
Rust ignores SIGPIPE, so `flexwm msg windows | head -1` dies with
`failed printing to stdout: Broken pipe` (exit 101) instead of exiting
quietly. `main.rs`'s graceful `Err` handling only covers the `write_all`
path at `msg.rs:34`; the `println!` path bypasses it. Compositor
genuinely unaffected — this is purely the CLI client.

## Why it matters

`flexwm msg windows | head` / `| jq ... | head` is the normal
agent-pipeline shape (and the human one) — every truncated read panics.
Standard Unix tools exit 0/quietly on a closed pipe; the fix is the
standard one (handle EPIPE on the print path the way the write path
already does).

## The fix, and what it needs

Route the reply print through the same graceful-EPIPE handling as the
request write (or restore a SIGPIPE disposition for the client binary),
plus a test that closes the reader mid-reply and asserts a clean exit.
Small, well-understood; no protocol, IPC, or compositor behavior changes.

## Resolution

Explicit per-write EPIPE handling, not a restored SIGPIPE disposition: a
new `crates/flexwm/src/output.rs` (`print_line` / `write_str` /
`write_bytes` map `BrokenPipe` to quiet success, exit 0; `warn` makes
stderr writes infallible), used at all six client-binary stdio sites —
the reply print, the screenshot-summary print, the screenshot-bytes
`write_all` (whose `Err` previously surfaced as a noisy `flexwm: Broken
pipe` exit 1), the `Warning` eprintln, `--help`, and the final error
report in `main`. Three reasons against SIGPIPE restoration, recorded in
the module doc: the disposition is process-wide and this binary also hosts
the compositor (which must never die to a signal because one peer stopped
reading); it needs unsafe plus `libc` on a binary that otherwise builds
dependency-free on macOS; and quiet exit 0 is the established Rust CLI
convention (ripgrep, fd, bat) for a consumer that got what it wanted. A
dead *socket* peer stays an ordinary error (exit 1) — EPIPE is mapped only
at the stdio edge, never inside `flexwm_ipc::Client`.

No README change: the only documented exit-status contract ("error
responses print and exit non-zero") is unchanged. No protocol, IPC, or
compositor behavior changes.

## Evidence

- Fail-first: `crates/flexwm/tests/msg_broken_pipe.rs` (fake Unix-socket
  server + the real binary, ~1 MiB reply so a sub-pipe-buffer write cannot
  weaken it) fails pre-fix with exit 101 on both truncated paths
  (`closed_stdout_on_a_large_reply_exits_quietly`,
  `closed_stdout_on_help_exits_quietly`) and passes post-fix with the
  full-read path (`a_full_read_is_unchanged`) green throughout.
- `cargo test -p flexwm`: 18 passed (macOS) / 700 passed, 1 ignored (dev
  VM, Linux). `cargo nextest run --workspace`: 117 passed (macOS) / 799
  passed, 1 skipped (dev VM). `cargo clippy -p flexwm --all-targets --
  -D warnings` and `cargo fmt --check -p flexwm`: clean on both.
- `scripts/smoke-test.sh` (default `--headless`, dev VM): exit 0.
- Live against a real headless compositor (dev VM): `msg screenshot |
  head -c0`, `msg windows | head -c0`, `--help | head -1`, `msg version |
  head -c0` all exit 0; `msg screenshot > file` byte-matches `msg
  screenshot --out file` (valid 1600x1000 PNG). Caveat, stated plainly:
  the live screenshot reply is 33 KiB (under the 64 KiB pipe buffer), so
  those runs may never have taken a real EPIPE — the guaranteed-EPIPE
  proof is the integration test above, which ran on the VM too.
