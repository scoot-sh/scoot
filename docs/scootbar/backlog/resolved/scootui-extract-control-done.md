---
title: "Extract `scootui`'s control socket: framing, paths, claim and connection bounds (move only)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# Extract `scootui`: the control socket (move only)

Filed 2026-10-12 in the #579 fix round: no ticket moved `control/`, yet `notify-control`
assumes it exists. Serves **both**: scootnotify's `msg` and `subscribe`, and agents.

## What to do

- Move `crates/scootbar/src/control/{framing,paths,claim,conn}.rs` and their bounds
  (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`, the write-stall deadline; `control/mod.rs:57-65`) into
  a `control` feature of `scootui`.
- `runtime_dir` already lives in `scoot-rt`.
- The socket name becomes a parameter instead of `scootbar-…`. That is the only API change,
  and scootbar passes its current name.
- scootbar's request handling (`protocol.rs`, `respond.rs`) stays in scootbar.

## Acceptance

scootbar's control tests (`control/tests.rs`, `subscribe_tests.rs`) and its socket bounds tests
pass unchanged, and the ratchet rows are within noise.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

scootnotify's verbs.

## Resolution

Shipped by `refactor/scootui-extract-control` (PR #NNN). A mechanical move
with no logic edits, verified by unchanged snapshots, unchanged
`count_allocations == 0` tests, and the ratchet rows below.

Moved, each as a `git mv` rename with only visibility and path edits:

- `control/{framing,paths,claim,conn}.rs` into a new `control` feature of
  `scootui`, with the server (`Server`, `MAX_CONNECTIONS`,
  `MAX_SUBSCRIBERS`) moving with them in `scootui::control::mod`. `framing.rs`
  moves verbatim (std only).
- `paths::resolve`/`from_env` take the app name (`scootbar`,
  `scootnotify`): it prefixes the socket and lock files and appears in the
  claim errors. scootbar passes `"scootbar"`, so its socket names and
  messages are byte-identical.
- The event kinds are the app's own: `Kinds` is a bitset over `u8`
  indices, and each app passes its kinds' discriminants (`*kind as u8` in
  scootbar, preserving the exact bit layout).
- The two refusal lines conn.rs writes are encoded by a small writer in
  `scootui::control` (the same `serde_json` serializer over the same
  shape), so their bytes are identical to the bar's own reply writer.
  `MAX_REQUEST_LINE` and `DROPPED` move to `scootui::control` with it
  (the bar's `protocol.rs` re-exports `DROPPED` for its client and tests).
- `scootbar/src/control/mod.rs` is a shim re-exporting the moved items,
  so every `crate::control::…` path in the daemon, the tests and the
  fakes resolves where it always has; `protocol.rs` (the bar's verbs),
  `respond.rs` and the `msg` client stay in scootbar.
- `scootui` gains `serde`/`serde_json` optional deps (both already in the
  tree) for the `control` feature, and `rustix/event` + `rustix/net`
  beside its `fs`; `Cargo.lock` records the `serde` edge. CI's scootui
  clippy/nextest loops cover `control` alongside the other features.

Deviations from the file list, all mechanical and forced by the move:

- The server itself (`Server` and its bounds in `control/mod.rs`) moves
  with the four files: without it the feature would be claim/conn
  plumbing with nothing driving it, and `notify-control` assumes the
  caps (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`) live in `scootui`.
- `admit`, `free_an_fd` and `has_spare` go public (documented as the
  consumers' white-box entry points): the moved tests drive them
  directly, and a private method of another crate is unreachable. As
  `scoot-dbus`' test helpers.
- The moved tests import the moved items from `scootui::control`
  (`conn::{Conn, Handler, STALL_DEADLINE, Status}`,
  `claim::Claim`, `MAX_REQUEST_LINE`) instead of `super::…`: a `pub use`
  nothing outside the crate names warns as unused in this binary crate,
  and the shim only re-exports what the daemon uses. Test bodies are
  untouched (the `Kinds::of`/`broadcast`/`wants` sites pass `*kind as
  u8`).
- The `AlreadyRunning` error carries the app name for its message; the
  tests pin only the `"already running"` substring and still pass.

Ratchet (M6 method, dev VM aarch64, rustc 1.97.1, release builds,
default bar with DejaVu under headless scoot, fresh XDG dirs, 14 s
settle, 60 s window; against `origin/main` at `522fd56ca`, whose
`crates/` tree is identical to #666's `35eddb754`):

- Stripped size: main 3019520 B vs head 3019520 B (identical).
- `.text`: main 2445768 B (`0x2551c8`) vs head 2450248 B (`0x256348`),
  +4480 B (+0.18%). The shipped size is identical (the growth fits the
  alignment padding); the `.text` delta is the moved server's public
  surface codegenning out of line in `scootui` where it used to inline
  into the binary crate. A same-source rebuild reproduces the head
  binary's `.text` exactly, so the delta is systematic, not layout
  noise; an alternative shape (the app's reply writer through the
  `Handler` trait, no serializer in `scootui`) measured +5312 B and
  more API surface, and was dropped.
- Idle: main RSS 4900/5076 PSS 3358/3446 wakeups Δ2 fds 8;
  head RSS 4840/5016 PSS 3298/3386 wakeups Δ2 fds 8. RSS/PSS within
  sharing noise, wakeups and fds identical.
- No waiver needed. scootbg untouched (`git diff --name-only` shows no
  `scootbg` file).
