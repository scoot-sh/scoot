---
title: "Audit that every fd the compositor holds carries close-on-exec — RESOLVED (all sources verified; spawn pin extended)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Audit that every fd the compositor holds carries close-on-exec — RESOLVED (all sources verified; spawn pin extended).

## The entry as filed

`docs/backlog/security/spawn-fd-cloexec-audit.md` (LOW):

> Split out of [the `O_CLOEXEC` no-op
> verdict](./tty-o-cloexec-noop-done.md) (2026-09-18) ... `State::spawn`
> ... inherits *any* fd without close-on-exec ... So spawn-time fd hygiene
> rests entirely on every fd source setting the bit ...
>
> Known-good, needing no re-verification: seatd-obtained DRM and input fds
> ...; sockets Smithay and std create ...; memfds ...
>
> What the audit needs is the rest of the inventory, verified at creation --
> atomically, not via a later `fcntl` ... Each source either sets the bit at
> creation (fix the straggler there) or gets a reason recorded why it cannot
> leak. Extend the spawn pin or add source-level pins for whatever moves.

## Verdict: all sources check out. No fix; the spawn pin is extended.

Every fd source in the compositor was verified — at creation in source
where the flag is visible in code, against the pinned dependency sources
where a library creates the fd, and live on the dev VM where only
measurement can answer (C libraries). Nothing was missing the bit, so no
production code changed. What landed is the pin extension the ticket asked
for: the spawn test now covers the socket, `try_clone` and eventfd marker
shapes end to end, each proven sensitive by neutering.

## The per-source table

Confidence ladder, per the ticket: measured > documented > assumed. Every
row below is measured or code-visible; nothing is assumed. "Code-visible"
means the `CLOEXEC` (or equivalent) flag is literal at the creation call in
the cited source. "Measured" means observed live on the dev VM on the
audited build (fdinfo `flags` carrying octal `02000000`, and/or an
end-to-end spawn child proving non-inheritance).

Pinned versions this was verified against: Smithay `0ff0098`, calloop
0.14.4, polling 3.11.0, wayland-backend 0.3.17, wayland-server 0.31.14,
Rust `input` 0.10.0 (libinput 1.31.3), Rust `udev` 0.9.3 (systemd/udevadm
261), Rust `libseat` 0.2.4 against system libseat 0.9.3 (seatd 0.9.3),
rustix 1.x, xcursor 0.3.11, rustc 1.97.1 on the dev VM (the probe and the
compositor build use the same toolchain).

| # | fd source | creation | CLOEXEC mechanism | confidence |
|---|-----------|----------|-------------------|-----------|
| 1 | Wayland listener socket | std `UnixListener::bind`, via wayland-server `ListeningSocket::bind_auto` (`socket.rs`) | `SOCK_CLOEXEC`, std | measured live (fd 10, `02004002`; `ss` shows it LISTENing on the wayland socket) + single-threaded std probe |
| 2 | Wayland socket lockfile | std `File` in `ListeningSocket::bind_absolute` | `O_CLOEXEC`, std | measured live (fd 9, `02400002`) + probe |
| 3 | Accepted Wayland client sockets | std `accept` (accept4 `SOCK_CLOEXEC`) | std | measured live (fds 14/25, `02000002`/`02000002`, with a live `foot`) |
| 4 | IPC listener socket | std `UnixListener::bind` in `ipc/listener.rs::stage` | `SOCK_CLOEXEC`, std | measured live (fd 24, `02004002`; `ss` shows it LISTENing at the `/proc/self/fd/<n>/s` staged path — the staging design confirmed live) + probe |
| 5 | IPC staging directory fd | `OpenOptions` with `O_DIRECTORY \| O_NOFOLLOW` (std, transient at startup) | `O_CLOEXEC`, std | probe (dir-`open` carries the bit); lifetime ends inside `stage()`, never reaches a spawn |
| 6 | IPC accepted streams | std `accept`, drained by `ipc/accept.rs` | std | probe (same call as #3) |
| 7 | Spare fds, IPC + Wayland (2 held) | `File::open("/dev/null")` at `Spare::new`; re-arm is raw `libc::open(O_RDONLY \| O_CLOEXEC)` (`ipc/accept.rs:228`, `wayland_accept.rs:243`) | explicit + std | measured live (fds 11/23, `02400000`) + probe |
| 8 | wayland-backend server epoll (`Display`) | `epoll::create(CreateFlags::CLOEXEC)` (`rs/server_impl/common_poll.rs:38`) | explicit, atomic | measured live (fd 6, `02000002`) + source |
| 9 | SCM_RIGHTS receipt — shm pool fds, dmabuf plane fds, data-device pipes | `recvmsg(DONTWAIT \| CMSG_CLOEXEC)` (`rs/socket.rs:77`; the `fcntl_setfd(CLOEXEC)` loop at `:97` is macOS/Redox-only, so on Linux the guarantee is `CMSG_CLOEXEC` alone) | explicit, atomic at receive | measured live (foot shm memfds `02400002`) + source. This is the row the whole client-fd inventory rests on. |
| 10 | calloop `Poll` (epoll + notifier eventfd + timerfd) | polling 3.11.0: `epoll_create1(CLOEXEC)`, `eventfd(CLOEXEC \| NONBLOCK)`, `timerfd(CLOEXEC \| NONBLOCK)` | explicit, atomic | measured live (fds 3/4/5) + source |
| 11 | calloop channel ping — screenshot completion channel, libseat session notifier | `eventfd(0, CLOEXEC \| NONBLOCK)` (`sources/ping/eventfd.rs`) | explicit, atomic | measured live (fds 12 session channel, 17/28 screenshot channel — the screenshot added exactly one eventfd, causal) + source |
| 12 | calloop `Timer` sources (frame tick, stall deadlines, lock timeout) | no fd: `TimerWheel`, in-process, woken by poll timeout | n/a — nothing to leak | source (`sources/timer.rs` creates no fd) |
| 13 | std `mpsc` job queue + `thread::spawn` (screenshot worker) | no fds: futex-based parker on Linux | n/a | measured (single-threaded probe: channel pair + spawn grows `/proc/self/fd` by 0) |
| 14 | `UnixStream::try_clone` (parked screenshot reply socket, `PendingIdle` socket) | `F_DUPFD_CLOEXEC`, std | std, atomic | probe + new end-to-end marker (below) |
| 15 | seatd DRM + input fds | libseat `MSG_CMSG_CLOEXEC` receive (`common/connection.c: `recvmsg(..., MSG_DONTWAIT \| MSG_CMSG_CLOEXEC)`, re-verified in upstream source) | explicit, atomic | measured live (`02504002` on `/dev/dri/card0` + all four `/dev/input/event*` — re-confirms the 2026-09-13 measurement on the current build) + upstream source |
| 16 | libseat seatd-connection socket | `socket(AF_UNIX, SOCK_STREAM \| SOCK_NONBLOCK \| SOCK_CLOEXEC, 0)` (upstream `libseat/backend/seatd.c: seatd_connect`; builtin backend `socketpair` same flags) | explicit, atomic | measured live (fd 13, `02004002`; `ss` shows it ESTABlished to `/run/seatd.sock`) + upstream source |
| 17 | Smithay shm-pool hold (`InnerPool { fd: OwnedFd }`) | wraps the received fd; only `mmap`s it (`wayland/shm/pool.rs`) — no new fd | inherits #9 | source + measured (the live pool memfds are #9's) |
| 18 | Smithay dmabuf hold (planes `Arc<OwnedFd>`, `map_plane` → `mmap`) | no dup, no new fd (`backend/allocator/dmabuf.rs`) | inherits #9 | source. `PixmanRenderer::import_dmabuf` maps and syncs only. |
| 19 | Smithay sealed memfds — dmabuf format table + keyboard keymap | `memfd_create(CLOEXEC \| ALLOW_SEALING)` (`utils/sealed_file.rs`) | explicit, atomic | measured live (fds 7/8, `02400002`) + source |
| 20 | flexwm nested host-side shm memfd | `memfd_create("flexwm-nested", CLOEXEC)` (`nested/buffers.rs:138`) | explicit, atomic | code-visible (the only production `memfd_create`; every other site in the tree is `*/tests*`, all with the flag — exhaustively grepped) |
| 21 | Smithay `DrmDeviceNotifier` | registers the existing DRM fd with epoll — no new fd | n/a | source |
| 22 | flexwm `UdevBackend` drm monitor | libudev netlink socket, held as `BorrowedFd` passthrough (`udev-0.9.3/monitor.rs`, no dup) | libudev's `SOCK_CLOEXEC` socket | measured live (fd 22, `02004002`, in the process netlink table as `NETLINK_KOBJECT_UEVENT`) |
| 23 | libinput context (epoll + timerfd) and its udev monitor | libinput C library; Smithay holds `context.as_fd()` passthrough, no creation | libinput/libudev | measured live (fds 15/16 `02000002`/`02004002`, fd 17 netlink `02004002`; attributed by elimination — the only creator in that init window is `Libinput::new_with_udev` + `udev_assign_seat` — with flags measured regardless) |
| 24 | config file read | `fs::read_to_string`, transient (open-read-close inside the call) | n/a — no persistent fd; std sets the bit anyway (probe) | source |
| 25 | cursor-theme files | `fs::read` per shape incl. lazy loads — each open-read-close, never held | n/a | source (`cursor/theme.rs:188`); `xcursor::CursorTheme` holds `Vec<PathBuf>`, no fds (xcursor-0.3.11 source) |
| 26 | gamma path (`set_gamma`, `gamma_size`) | ioctls on the DRM fd — no `open` in `gamma_control.rs` | n/a | source (grep: no file/socket creation) |
| 27 | `fd_pressure::read_dir`, `dispatch.rs` probe `mmap` | transient iterator / mapping, no persistent fd | n/a | source |
| 28 | dmabuf `sync_plane` ioctls | no fd | n/a | source |
| — | `flexwm msg` client stdio (`output.rs`, `msg.rs`) | a different process (the client binary), never inherited by compositor children | out of scope | source |
| — | test-only fds (`libc::pipe` in `gamma_control/tests.rs`, `/dev/udmabuf` opens in `dmabuf/tests.rs`) | tests never spawn (except `spawn.rs`, which asserts only about its own held-open markers) | out of scope; the udmabuf helper sets `O_CLOEXEC` + `UDMABUF_FLAGS_CLOEXEC` anyway | source |

Attribution notes for the live tables (both runs, 2026-09-18, dev VM,
binary built from `44192c2` — the branch carries no production changes, so
the binary is the audited code):

- Headless (`FLEXWM_SOCKET=/tmp/fdaudit.sock --headless` + `msg
  screenshot` + IPC-spawned `foot`): 15 non-stdio fds — 2 epolls, 2
  eventfds, 1 timerfd, 2 sealed memfds, lockfile, 2 listener sockets, 2
  spares, 1 accepted client socket, 2 foot shm memfds, 1 channel eventfd.
- `--tty` (SSH-started, seatd 0.9.3, `/dev/dri/card0`): 22 non-stdio fds —
  the headless set plus session-channel eventfd (12), seatd connection
  (13), DRM (14), libinput epoll+timerfd (15/16), libinput udev monitor
  (17), four input devices (18–21), drm udev monitor (22). Screenshot +
  foot added the accepted socket (25), two shm pools (26/27) and the
  completion channel (28), exactly as predicted.
- Every non-stdio fd in both tables reports the `02000000` bit in
  `/proc/<pid>/fdinfo/<n>` `flags` (fd-numbered tables summarized above;
  PR #124 review independently re-ran both end-to-ends live at the merge
  HEAD and confirmed zero inheritance on both backends).
- End-to-end (both backends): an IPC-spawned child listing `/proc/self/fd`
  holds only stdio plus its own listing fd — zero of the compositor's
  15 (headless) / 25 (`--tty`) non-stdio fds appear, with targets
  readlinked to rule out number coincidence. The one curiosity (child
  stdout a `pipe:` neither parent holds) is created post-fork outside
  compositor code — no compositor `Stdio::piped`/redirection exists
  (grepped), and a leak would have to appear in the parent table too.

## What landed (pin extension, test-only)

`activation/tests/spawn.rs::a_spawned_child_inherits_no_close_on_exec_fd`
now holds four more owned markers across the real `State::spawn`, each
asserted absent from the child with premise (`F_GETFD` carries the bit)
and liveness (still open after the listing) guards:

- a connected `UnixStream::pair` (listener/accepted-stream shape),
- a `try_clone` of one end (parked-screenshot / `PendingIdle` shape),
- a `libc::eventfd(EFD_CLOEXEC | EFD_NONBLOCK)` (calloop channel-ping shape).

Sensitivity, each proven by clearing that marker's bit post-premise and
watching exactly it appear in the child (neuter lines since removed):

- clone neutered → `inherited a close-on-exec cloned socket (fd 16):
  [0, 1, 13, 16, 2, 3]` — red.
- pair end neutered → `inherited a close-on-exec socket pair end (fd 14):
  [0, 1, 13, 14, 2, 3]` — red.
- eventfd neutered → `inherited a close-on-exec eventfd (fd 17):
  [0, 1, 13, 17, 2, 3]` — red.
- Un-neutered: green.

## Residuals, stated plainly

- libinput's and libudev's own creation flags were verified by live
  measurement on the exact linked versions, not re-derived from their
  upstream sources (freedesktop's gitlab is bot-walled; the systemd source
  was not chased further once every fd it could have created was measured
  with the bit set). A future libinput/libudev that dropped `CLOEXEC`
  would now be caught two ways: this audit's live tables would change, and
  the extended spawn pin covers the socket/eventfd shapes (not the
  epoll/timerfd/netlink shapes, which have no std-constructible marker —
  that is the remaining narrow gap, accepted: those fds come only from C
  libraries whose behavior is measured here).
- The live runs used the prebuilt debug binary from `44192c2`; the branch
  adds no production code, so the audited code and the measured code are
  the same. Full standard set green on the final tree (exact commands and
  raw outputs in the PR).
- No README change: no user-facing surface (internal audit + test).
