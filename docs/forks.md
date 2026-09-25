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
| [`scoot-sh/smithay`](https://github.com/scoot-sh/smithay/tree/scoot/xwayland-selection-dnd) | [Smithay/smithay](https://github.com/Smithay/smithay) | `0ff00983` (master, 2026-09-09) | `43f50eb2`: a `Drop` for the imported syncobj timeline; then eleven XWayland selection and drag commits, `35c335e0`..`53aafc36` (see below) | **yes**, `crates/scoot/Cargo.toml` rev `53aafc36` (PR #246, XWayland Phase 4; `43f50eb2` since PR #233) | Without the first, every explicit-sync timeline import leaks a kernel syncobj handle until scoot exits (~24 MB/s from a looping client, unaccounted slab). Without the rest, large clipboard transfers between X and Wayland are cut to 64 KiB, a stuck X reader makes scoot buffer a whole Wayland selection, transfers either way pile up without bound or stall for good, and scoot cannot gate who serves a paste or starts a drag. |
| [`scoot-sh/wayland-rs`](https://github.com/scoot-sh/wayland-rs/tree/scoot/server-fd-queue-cap-adaptive) | [Smithay/wayland-rs](https://github.com/Smithay/wayland-rs) | `72f7fe0d` (the wayland-backend 0.3.17 release, `v0.31.x` branch) | `a39311b8`: server side, disconnects a client leaving too many received fds unclaimed; `70f81e00`: sizes that cap at one eighth of the soft `RLIMIT_NOFILE`, 128..=1024 | **yes**, root `Cargo.toml` `[patch.crates-io]` rev `70f81e00` (PR #241) | wayland-backend queues fds a client sends with fd-less requests for the connection's life, so one idle client could fill scoot's fd table and shed every newcomer, `scootctl` included. |

## Per fork

### `scoot-sh/smithay`

- **Branch:** `scoot/xwayland-selection-dnd`, twelve commits on `0ff00983`.
  Its first, `43f50eb2`, is also the tip of `scoot/syncobj-timeline-drop`,
  which PR #233 pinned; that branch is kept as it was, and nothing pins it
  now. The XWayland commits, in order, each measured before it was written
  (fail-first records on the dev VM, `~/evidence/xw4/`; see
  `docs/backlog/protocols/xwayland-support.md`'s Phase 4 record):
  - `35c335e0` **pace incoming INCR transfers on the write.** An X
    selection larger than one chunk was read with `delete=true` and deleted
    again after the write; a prompt owner's next chunk landed between the
    two and was deleted unread, so a 2 MiB X selection pasted into Wayland
    as exactly 65536 bytes. Now read without deleting; the delete after the
    write (flushed) asks for the next chunk, which also paces the owner
    against a slow reader.
  - `b6bcc47d` **a blocking pipe for the Wayland source.** The pipe an X
    read is served through was `O_NONBLOCK` on both ends, and the write end
    goes to the Wayland client; one writing with blocking `write(2)` got
    `EAGAIN` once the pipe filled (64 KiB of 2 MiB reached X).
  - `f864d843` **backpressure on outgoing INCR.** The Wayland source was
    read level-triggered regardless of the X requestor, so a requestor that
    never took a chunk made scoot buffer the whole source (measured: a
    64 MiB selection, all of it, at `b5aa306f`, the pre-rebuild twin of
    `b6bcc47d` kept reachable as tag `evidence/xw4-pre-rebuild-b5aa306f`).
    Reading now pauses at two chunks until the requestor's next delete.
  - `a1ef7fe7` **`X11Wm::selection_owner`.** A Wayland paste is converted
    from whoever owns the X selection now, and an owner that never answers
    `TARGETS` takes it silently; scoot needs the tracked owner to serve a
    paste only from the one its gate accepted.
  - `853d305f` **`XwmHandler::allow_drag`.** The window manager turned any
    held press into a drag for any X client taking `XdndSelection`; the hook
    lets scoot apply its drag serial check. Default allows, as before.
  - `4aca6ef5` **bound transfers waiting on an X owner.** Each Wayland read
    of an X selection held an fd and a window until answered; a silent
    owner leaked one per read. Pending conversions are dropped when the
    selection changes hands, and a read past 8 in flight is refused
    (`SelectionError::TooManyTransfers`).
  - `0d281abf` **flush a new Wayland selection to the X server.** The
    ownership change sat unsent, so `xclip -o` right after `wl-copy` read
    the previous owner.

  Four more from the review of PR #246, each against the reviewer's probe
  first (`~/evidence/xw4/review/`, baseline at scoot `4c423b4` on
  `0d281abf`, then at the fixed head):
  - `567ac2cc` **count each selection's ownership changes**
    (`X11Wm::selection_generation`). `SetSelectionOwner` accepts any window
    id, so comparing owner windows missed a background client taking the
    clipboard under the approved owner's own window (it served the next
    Wayland paste); any change of hands now moves the count.
  - `9cc46d1a` **stream incoming properties in bounded slices** -- a fix
    forward of `35c335e0`, whose non-deleting read re-read the whole
    property on every new value (16 × 1 MiB appends: RSS 30.6 → 303.5 MB,
    and the same bytes handed to the reader again and again). Properties
    are now read 64 KiB at a time, the next only once the last is written;
    an INCR chunk only once our delete is seen to take effect. That also
    bounds a single huge property (96 MiB built by appends: 30.6 → 128.4 MB
    before, 30.9 → 31.1 MB after).
  - `3c53776f` **bound the transfers X clients open out of a selection**:
    4 per X client (client bits of the requestor's window id), 16 per
    selection. One client's 300 requestor windows held 300 fds (26 → 326);
    after, 26 → 30.
  - `53aafc36` **drop transfers that will not finish**: a closed reader
    (the pipe reports hang-up), 30 s idle
    (`X11Wm::set_selection_transfer_timeout`), and on a change of owner the
    pastes stalled on the old one past a 1 s grace -- moving ones kept. The
    8-paste bound had been exhausted for good by an owner going quiet
    mid-INCR; after, a new owner's paste works.
- **Evidence:** `docs/backlog/resolved/syncobj-handle-leak-done.md`, and on the
  dev VM `~/evidence/sync/master-validation/`. Upstream master `79bbed5e1`
  (2026-09-22) was built and measured: it leaks 3.5–4.1 MB per test run,
  against about zero with the fork.
- **Upstream status (last checked 2026-09-25, master `79bbed5e1`):** all
  twelve unfixed on master (the XWM code carries the double delete, the
  `O_NONBLOCK` pipe, the unpaused read, the unflushed owner change and no
  hooks). No issue or PR exists. Nothing has been filed from here.
- **Upstream policy note, for the maintainer's decision:** Smithay's
  `AI.md` asks contributors to disclose AI-generated code, discourages
  it, and asks for human-written issue and PR text. Its `DCO.md` requires
  the contributor's own certification.
- **Drop the fork when** an upstream rev carries equivalents of every
  commit (or scoot stops needing one): `docs/backlog/core/smithay-fork-repin.md`.
- **What scoot relies on:** `compositor/xwayland/tests/clipboard.rs` and
  `dnd.rs` fail if any of the XWayland commits is lost to a repin (each was
  written against a failing test; the flush one is a race, and failed 5 of
  6 runs without the commit); the syncobj commit's own measurement is in
  its resolved record.

### `scoot-sh/wayland-rs`

- **Branch:** `scoot/server-fd-queue-cap-adaptive`. Its first commit,
  `a39311b8`, is also the tip of `scoot/server-fd-queue-cap`, which PR
  #241 first pinned with a fixed cap of 128; that branch is kept as it was
  (its history is not rewritten), and nothing pins it now.
- **Evidence:** `docs/backlog/resolved/wayland-backend-fd-queue-done.md`,
  and on the dev VM `~/evidence/fdq/`. The route was chosen after
  scoot-side alternatives (per-client attribution, a kill heuristic, a
  socket proxy) were ruled out.
- **Why the cap is adaptive (`70f81e00`):** the check runs before each
  read, so it counts fds a client has sent ahead of the requests that
  claim them, and well-behaved clients get that far ahead: any flush
  carrying more than 28 fds sends them 28 per `sendmsg` with one byte each,
  ahead of the bytes. A client on `wayland-client`'s pure-Rust backend does
  it for every flush, and a stock libwayland client (1.26) does it once its
  socket has filled and its unbounded buffers have grown: review of PR #241
  measured one stalled behind a stopped compositor disconnected at 140 fds
  under the fixed 128, while 0.3.17 served 600. (`a39311b8`'s doc comment
  claimed libwayland clients never come near the cap; that was wrong, and
  `70f81e00` replaces it.) The cap is now libwayland-server's own bound,
  1024 (its `fds_in` ring holds 4096 bytes of fds by default), wherever
  the table allows it: one eighth of the soft limit, read when each client
  is created, clamped to 128..=1024. scoot raises its soft limit at startup
  to min(hard limit, 65536) (`crates/scoot/src/compositor/nofile.rs`),
  so the cap is 1024 wherever the hard limit is 8192 or more. Where the hard
  limit is 1024 (a container) it stays 128, the startup log says so, and a
  stalled libwayland client there can still be disconnected past about 128.
- **How it is pinned:** a `[patch.crates-io]` entry in the root
  `Cargo.toml`, because Smithay, `wayland-server` and `wayland-client` all
  depend on `wayland-backend` from crates.io. `wayland-sys` moves to the
  fork's source with it (a path dependency inside that repository); the
  fork leaves it byte-identical to the 0.3.17 release. Both are covered
  by one `flake.nix` `outputHashes` entry, `wayland-backend-0.3.17`.
- **What scoot relies on:** at most the cap in unclaimed received fds per
  connection (up to 30 more for a moment inside one read), pinned against
  the real backend at whatever limit the test process runs with by
  `crates/scoot/src/compositor/fd_pressure/tests/backend_queue.rs`, which
  fails if the patch is lost to a repin, `cargo update` or rebase;
  `backend_queue_client.rs` pins the legitimate shapes (the backpressure
  case served at the cap, a Rust client's one-flush batch of 1036 served).
  `fd_pressure.rs` adds both figures to its arithmetic, on both tables.
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
