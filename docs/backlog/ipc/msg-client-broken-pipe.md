---
title: "`flexwm msg` panics when its stdout reader goes away (EPIPE)"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# `flexwm msg` panics when its stdout reader goes away (EPIPE)

Found 2026-09-17, reviewing PR #61
(`resolved/screencopy-shell-thumbnails-fallback-done.md`), filed rather
than fixed inside it.

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
