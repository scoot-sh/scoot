---
title: "The crate, the daemon and the control socket"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# The crate, the daemon and the control socket

The first real PR, and the one that creates the crate.

- Add `crates/scootbg` (the workspace's `crates/*` glob picks it up),
  MIT, `description` saying "wallpaper daemon for Wayland" (not
  "compositor": it is a client). A short `crates/scootbg/README.md` may
  point at `docs/scootbg/`, which stays the home of its docs.
- Add `crates/scootbg-mem` beside it: the only `unsafe` in scootbg.
  - Two modules: the large-allocation global allocator and the `wl_shm`
    buffer mapping (sealed memfd). Each `unsafe` block carries a
    `// SAFETY:` comment, enforced by clippy's
    `undocumented_unsafe_blocks` lint.
  - Pure Rust (`rustix`, no `libc`), `publish = false`.
  - `scootbg` is `#![forbid(unsafe_code)]`.

  See the
  [design and safety arguments](resolved/dependencies-done.md#11-scootbgs-own-unsafe-two-modules-in-one-small-crate).
- One binary with subcommands: `daemon` runs the Wayland client, and every
  other subcommand is a client of its control socket.
- Control socket at `$XDG_RUNTIME_DIR/scootbg-$WAYLAND_DISPLAY.sock`, so
  two sessions (a nested scoot inside another) never share one.
  `WAYLAND_DISPLAY` may be an absolute path, which libwayland allows, so
  derive the name from its final component (or a hash) rather than
  splicing it in. Line-framed
  JSON requests and replies, versioned from day one like `scoot-ipc`.
  Self-contained, not reusing `scoot-ipc`: it would pull no compositor
  types, but it would share only ~25 lines of generic framing and would
  put `crates/scoot-ipc/` on scootbg's CI path
  ([decided](resolved/dependencies-done.md#5-serialization-control-socket-and-state-file)).
- Dependencies as decided in
  [`resolved/dependencies-done.md`](resolved/dependencies-done.md), each
  with a one-line reason in `Cargo.toml` comments linking there. Weigh the
  release binary with `cargo build --release -p scootbg`, not a
  `--workspace` build (feature unification).
- A second `scootbg daemon` on the same display refuses loudly (socket
  already live) instead of stacking a second set of surfaces; a stale
  socket from a crashed daemon is detected and replaced.
- A Wayland disconnect (compositor exit) ends the daemon cleanly with a
  non-zero status, never a panic.
- Nix: a `scootbg` package output of its own (the `scoot` package stays
  `scoot` only, as with `scootctl`) and a place in the dev shell.
- CI, split by path, extending the `changes` job in
  `.github/workflows/ci.yml` (which today only skips docs-only PRs):

  | Changed | Runs |
  |---|---|
  | `crates/scoot/`, `crates/scoot-core/`, `crates/scoot-ipc/`, `crates/scootctl/`, `scripts/smoke-test.sh`, `vm/compositor-deps.nix` | the scoot jobs |
  | `crates/scootbg/`, `crates/scootbg-mem/` | a scootbg job (fmt, clippy, tests) |
  | `Cargo.toml`, `Cargo.lock`, `flake.*`, `nix/`, `.github/` | everything |
  | anything not listed (`.config/nextest.toml`, `resources/`, `devenv.*`, other `scripts/` and `vm/` files, `LICENSE`) | everything |
  | either side | the scootbg-on-headless-scoot integration test |

  Match on directory prefixes with the trailing slash: `crates/scoot*`
  would also match `crates/scootbg`, and `crates/scootbg/` does not match
  `crates/scootbg-mem/`, which is why both are listed.

  The integration test runs for both because the `apply-config` contract
  couples them: a scoot change can break scootbg's end-to-end run and the
  reverse. Gate jobs with `if:` on the `changes` outputs, never a
  workflow-level `paths:` filter (a required check would wait forever),
  and keep pushes to `main` running everything.

Done when `scootbg daemon` connects, binds its globals, answers `query`
with an empty output list shape, and exits cleanly on `kill` and on
compositor exit, with tests for the socket lifecycle.
