---
title: "Split the CLI out of the compositor binary into `scootctl` — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Split the CLI out of the compositor binary into `scootctl` — RESOLVED

## What it said

`docs/backlog/resolved/rename-flex-family-done.md` (moved there on close;
kept under a stable filename so existing links keep resolving): `scoot msg …` is one `Command` variant of
the single `scoot` binary — the same executable that starts the compositor
also sends it IPC requests. A separate `scootctl` binary cleanly separates
"the compositor" from "a client of the compositor" (the computer-use goal:
an agent shells out to `scootctl`, not to the compositor's own binary). Open
questions it carried: what `scoot msg` becomes, where the macOS story lands,
and which crate `msg.rs` plus the `cli.rs` parsing moves to. A status bar
stays out of scope.

## Resolution

**New package `crates/scootctl` (lib + bin); `scoot msg` kept as a
permanent alias; no wire change, no `PROTOCOL_VERSION` bump** (still 2).

- **Kept, not removed.** `scoot msg` is the documented interface (README,
  `docs/ipc.md`, agent tooling, muscle memory); deleting it is user-facing
  harm with zero engineering gain. The separation is about where the code
  lives and what agents reach for, not about deleting a working entry
  point. A future removal (if ever) is its own migration ticket.
- **Lib+bin shape; `scoot` depends on the lib.** The msg grammar, the
  `Error` display strings (several byte-pinned by tests), and the help text
  have exactly one owner (`scootctl::cli`); both binaries are front-ends
  over it. `concat!` takes literals only, so the two full `--help` strings
  can't be composed from the shared `REQUESTS_HELP`/`ACTIONS_HELP`
  fragments at compile time -- instead a containment test on each side pins
  that both print the same blocks, and a 29-case unit test in `scoot` pins
  that `scoot msg ...` parses byte-identically to `scootctl ...` (same
  request *and* same error).
- **Moved:** `msg.rs` and `output.rs` whole (PR #62's EPIPE handling with
  its tests), the `message()` subtree of `cli.rs` (message/pointer/button/
  horizontal/vertical/`number`/key-combo parsing), the `Error` enum, and the
  msg-side `cli.rs` tests. **Stayed in `scoot`:** `RendererKind` (shared
  with `compositor::config` and render/state users), `CompositorOptions`,
  `MAX_OUTPUT_DIMENSION`/`MAX_OUTPUTS`, compositor-flag parsing,
  compositor-side tests. `scoot`'s `Command::Msg` arm is now one call into
  `scootctl::parse_msg` + `scootctl::run`, so the alias cannot drift.
- **One judgment call the code forced:** `compositor::config::parse_bind`
  reuses the action parser, which now lives in `scootctl` -- so the
  compositor reads `scootctl::action` rather than `crate::cli::action`.
  Dependency direction stays clean (`scoot` → `scootctl` →
  `scoot-ipc`; `scootctl` never touches the compositor), and the `[binds]`
  grammar is still one parser, not two.
- **macOS story:** `scootctl` builds everywhere with zero cfg gating (pure
  client: `scoot-ipc` + `serde_json` + std). `flake.nix` gains
  `packages.scootctl` (`-p scootctl`, so `$out/bin` carries only the
  client) and `apps.scootctl`; Darwin `packages.default` is now `scootctl`
  with the crate's own description (the old Darwin-conditional
  `meta.description` on the `scoot` package stays, still accurate for an
  explicit `.#scoot` build there); Linux default stays `scoot` (whose
  `$out/bin` now carries both binaries). The `#[cfg(not(linux))]`
  `start_compositor → Err` arm in `scoot`'s main stays.
- **`--print-default-config` noted, not built:** splitting the client out
  makes the compositor binary the obvious home for a future
  `--print-default-config`, but that is its own ticket, not a rider here.

## Evidence (dev VM, `ssh -p 2222 dev@localhost`, 9p mount at `/mnt/scoot`)

- `cargo nextest run --workspace`: **1156 passed, 4 skipped** (includes
  the moved `scootctl` unit + `msg_broken_pipe` suites and the new
  29-case alias-equivalence test).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
  `cargo fmt --check --all`: clean.
- `scripts/smoke-test.sh` (`SMOKE_PREFIX=/tmp/smoke-scootctl`, default
  `--headless`): green end to end, including the new equivalence section --
  `version`/`windows`/`outputs`/screenshot-PNG/`action` replies
  byte-identical between binaries, three malformed-arg failures identical
  in exit code and message.
- Bug-bash, live `--headless` session: no-listener connect error identical
  (`No such file or directory (os error 2)`, exit 1 both); closed stdout on
  a real reply exits 0 both; closed stderr exits 1 with no panic both;
  `--help` closed-stdout exits 0 both; screenshot `--out` summaries
  identical, PNGs byte-identical across binaries, stdout PNG matches file
  PNG for both; `$SCOOT_SOCKET` honored by both.
- `ldd` on both debug binaries: no `libgbm`/`libdrm`/`libEGL`/`libGLESv2`
  (`scootctl` links libc + libgcc only). CI asserts the same for both
  release builds.
- macOS (this machine): `cargo build/test/clippy/fmt -p scootctl` green;
  `nix flake show` lists `packages.scootctl` + `apps.scootctl` per system;
  `nix eval` confirms Darwin default is `scootctl` (crate's own
  description) and Linux default stays `scoot`; real `nix build .#scootctl`
  produced a working Darwin client (`--help`, connect-error path).
- No benchmark: arg parsing is a cold path (one process per invocation).

## Follow-ups (own tickets, not riders)

- `--print-default-config` on the compositor binary (home now obvious).
- Removing `scoot msg`, if ever (migration ticket with a deprecation
  window, not a silent deletion).
