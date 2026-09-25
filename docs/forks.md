# Dependency forks

scoot carries small fixes to its dependencies as **forks under the
`scoot-sh` GitHub org**, pinned by exact commit. Nothing is sent upstream
from this project: whether and when a fix is offered upstream is the
maintainer's decision, made later, per fork. This file is the list we
watch. Every fork added, changed or dropped updates it in the same PR.

Each fork is **one upstream commit plus the fewest possible commits on
top**, so it stays easy to review, rebase, and drop.

| Fork | Upstream | Based on | Carried commits | Pinned in scoot | Why |
| --- | --- | --- | --- | --- | --- |
| [`scoot-sh/smithay`](https://github.com/scoot-sh/smithay/tree/scoot/syncobj-timeline-drop) | [Smithay/smithay](https://github.com/Smithay/smithay) | `0ff00983` (master, 2026-09-09) | `43f50eb2`: a `Drop` for the imported syncobj timeline | **yes**, `crates/scoot/Cargo.toml` rev `43f50eb2` (PR #233) | Without it, every explicit-sync timeline import leaks a kernel syncobj handle until scoot exits (~24 MB/s from a looping client, unaccounted slab). |
| [`scoot-sh/wayland-rs`](https://github.com/scoot-sh/wayland-rs/tree/scoot/server-fd-queue-cap) | [Smithay/wayland-rs](https://github.com/Smithay/wayland-rs) | `72f7fe0d` (the wayland-backend 0.3.17 release, `v0.31.x` branch) | `a39311b8`: server side, disconnects a client leaving more than 128 received fds unclaimed | **yes**, root `Cargo.toml` `[patch.crates-io]` rev `a39311b8` (this PR) | wayland-backend queues fds a client sends with fd-less requests for the connection's life, so one idle client could fill scoot's fd table and shed every newcomer, `scootctl` included. |

## Per fork

### `scoot-sh/smithay`

- **Evidence:** `docs/backlog/resolved/syncobj-handle-leak-done.md`, and on the
  dev VM `~/evidence/sync/master-validation/`. Upstream master `79bbed5e1`
  (2026-09-22) was built and measured: it leaks 3.5–4.1 MB per test run,
  against about zero with the fork.
- **Upstream status (last checked 2026-09-24):** unfixed on master. No
  issue or PR exists. Nothing has been filed from here.
- **Upstream policy note, for the maintainer's decision:** Smithay's
  `AI.md` asks contributors to disclose AI-generated code, discourages
  it, and asks for human-written issue and PR text. Its `DCO.md` requires
  the contributor's own certification.
- **Drop the fork when** an upstream rev carries an equivalent fix:
  `docs/backlog/core/smithay-fork-repin.md`.

### `scoot-sh/wayland-rs`

- **Evidence:** `docs/backlog/resolved/wayland-backend-fd-queue-done.md`,
  and on the dev VM `~/evidence/fdq/`. The route was chosen after
  scoot-side alternatives (per-client attribution, a kill heuristic, a
  socket proxy) were ruled out. libwayland bounds the same queue, but its
  bound is 1024, which scoot's 1024-fd table reaches first.
- **How it is pinned:** a `[patch.crates-io]` entry in the root
  `Cargo.toml`, because Smithay, `wayland-server` and `wayland-client` all
  depend on `wayland-backend` from crates.io. `wayland-sys` moves to the
  fork's source with it (a path dependency inside that repository); the
  fork leaves it byte-identical to the 0.3.17 release. Both are covered
  by one `flake.nix` `outputHashes` entry, `wayland-backend-0.3.17`.
- **What scoot relies on:** 128 unclaimed received fds per connection at
  most (30 more for a moment inside one read), pinned against the real
  backend by `crates/scoot/src/compositor/fd_pressure/tests/backend_queue.rs`,
  which fails if the patch is lost to a repin, `cargo update` or rebase.
  `fd_pressure.rs` adds both figures to its reserve arithmetic.
- **Its one cost to legitimate clients:** the check runs before each read,
  so fds a client has sent ahead of their requests count. A client on the
  Rust `wayland-client` sends every fd past the last 28 of a flush ahead,
  so one that queues more than 140 fd-carrying requests between flushes is
  disconnected (served on 0.3.17). libwayland clients cannot hit it. The
  same tests pin 140 served and 141 refused.
- **Upstream status (last checked 2026-09-24):** unbounded in 0.3.17 and
  on master (the 0.4 rewrite). No issue or PR exists. Nothing has been
  filed from here. There is no AI-contribution policy file.
- **Drop the fork when** a released wayland-backend bounds the queue.

## Maintaining a fork

- Rebase the carried commit onto the new upstream base before any dependency
  bump, and update this table and the pin together.
- Verify claims about a forked dependency against **the fork rev's
  source**, not upstream knowledge.
- A Nix build pins git dependencies by hash (`flake.nix`
  `cargoLock.outputHashes`). Update the hash with the rev.
