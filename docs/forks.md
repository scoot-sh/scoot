# Dependency forks

scoot carries small fixes to its dependencies as **forks under the
`scoot-sh` GitHub org**, pinned by exact commit. Nothing is sent upstream
from this project: whether and when a fix is offered upstream is the
maintainer's decision, made later, per fork. This file is the list we
watch. Every fork added, changed or dropped updates it in the same PR.

Each fork is **one upstream commit plus the fewest possible commits on
top**, so it stays easy to review, rebase, and drop.

**A fork is the last resort.** Every fork, and every commit carried on one, has an
"alternatives considered" line in its entry below: the scoot-side routes weighed
(a handler or wrapper in scoot, an in-tree copy of the module, a workaround at
another layer), each with why it was rejected and the evidence. A hard constraint
is checked against the pinned fork's source. If the line is missing, the entry is
incomplete, and "the fork was simpler" is not a reason. (Rule added 2026-09-29; the
existing entries are being checked against it by
[the decision audit](backlog/core/fork-decisions-audit.md).)

| Fork | Upstream | Based on | Carried commits | Pinned in scoot | Why |
| --- | --- | --- | --- | --- | --- |
| [`scoot-sh/smithay`](https://github.com/scoot-sh/smithay/tree/scoot/cursor-dmabuf-storage) | [Smithay/smithay](https://github.com/Smithay/smithay) | `0ff00983` (master, 2026-09-09) | `43f50eb2`: a `Drop` for the imported syncobj timeline; then thirteen XWayland selection and drag commits, `35c335e0`..`5b575329` (see below); then `74edbf32`: clamp the pixman source image to its edge when scaling; then `6e6fe896`: flush the XDND proxy's remap when a drag leaves an X window; then `7388af13` and `9515d7e5`: end a Wayland drop onto X once, and end an offer whose target is gone or never finishes; then `d3a4cd73`: only a real X drag gives up a pending drop onto X; then `b1ac3ca7`: an X drag enters another client's X window without waiting for types (Smithay's generic `input/dnd` gains a defaulted `DndFocus::enter_needs_metadata`); then `7e18b661`: remap the proxy only once the drag has left every X window; then `b16cd6a2`: wait for types only over the window the drag started on; then `fcf6f314` (unused, see below); then `e7130254`: apply a commit's cached buffer scale and transform on every commit that has a buffer; then `035d447c`: flush XSETTINGS writes; then `7ab72d53`: scan out a compositor-owned dma-buf element | **yes**, `crates/scoot/Cargo.toml` rev `7ab72d53` (the drawn cursor on an overlay plane; `035d447c` was the XSETTINGS flush for scale-aware X windows; `e7130254` was the buffer-scale fix, content-identical to `6ab8b4a2` which `main` pinned from `scoot/buffer-scale-without-new-buffer` before this branch's XWayland lane merged; the same-app quick drag; `7e18b661` was the X drag first-motion fix, `b1ac3ca7` and its review fix; `d3a4cd73` was the XWayland pointer-focus X arm; `74edbf32` was PR #272; `5b575329` was PR #246, XWayland Phase 4; `43f50eb2` since PR #233) | Without the first, every explicit-sync timeline import leaks a kernel syncobj handle until scoot exits (~24 MB/s from a looping client, unaccounted slab). Without the rest, large clipboard transfers between X and Wayland are cut to 64 KiB, a stuck X reader makes scoot buffer a whole Wayland selection, transfers either way pile up without bound or stall for good, and scoot cannot gate who serves a paste or starts a drag. Without `74edbf32`, every upscaled surface fades to a semi-transparent 1-px border under the default renderer; without `6e6fe896`, an X drag that crossed an X window cannot drop on a Wayland one, so drags from X could not be let onto X windows; without `9515d7e5`, a Wayland drop onto an X target that dies or hangs before finishing stops every later X drag until scoot restarts, and without `d3a4cd73` any X client can end such a drop in flight and keep the selection; without `b1ac3ca7`, a GTK or Qt drag released on its first motion onto another X app drops nothing, and without `7e18b661` an X window mapped again under an X drag has the proxy over it, and without `b16cd6a2` a quick drag between two windows of one X app instance drops nothing; without `e7130254`, a scale-only commit keeps rendering at the old buffer scale (see below); without `035d447c`, an X app started right after XWayland is ready can read no scale over XSETTINGS and draw at half size; without `7ab72d53`, scoot's drawn cursor can never ride an overlay plane, so on a CRTC with no cursor plane (Apple's DCP) a visible pointer makes every fullscreen frame composite. |
| [`scoot-sh/wayland-rs`](https://github.com/scoot-sh/wayland-rs/tree/scoot/server-fd-queue-cap-adaptive) | [Smithay/wayland-rs](https://github.com/Smithay/wayland-rs) | `72f7fe0d` (the wayland-backend 0.3.17 release, `v0.31.x` branch) | `a39311b8`: server side, disconnects a client leaving too many received fds unclaimed; `70f81e00`: sizes that cap at one eighth of the soft `RLIMIT_NOFILE`, 128..=1024 | **yes**, root `Cargo.toml` `[patch.crates-io]` rev `70f81e00` (PR #241) | wayland-backend queues fds a client sends with fd-less requests for the connection's life, so one idle client could fill scoot's fd table and shed every newcomer, `scoot msg` included. |

## Per fork

### `scoot-sh/smithay`

- **Branch:** `scoot/cursor-dmabuf-storage`, twenty-six commits on
  `0ff00983`, the tip `7ab72d53` pinned: `scoot/xwayland-selection-dnd`
  (twenty-five, tip `035d447c`) plus the cursor commit, on a branch of its
  own until the PR adding it merges, after which the lane branch can be
  fast-forwarded to it. `fcf6f314`, the twenty-third, is
  carried unused (see below). `scoot/buffer-scale-without-new-buffer`
  (`6ab8b4a2` on `7e18b661`) was pinned briefly by the buffer-scale PR
  while this lane's XWayland commits were unmerged; nothing pins it now.
  Its first, `43f50eb2`, is also the tip of `scoot/syncobj-timeline-drop`,
  which PR #233 pinned; that branch is kept as it was, and nothing pins it
  now. The XWayland commits, in order, each measured before it was written
  (fail-first records on the dev VM, `~/evidence/xw4/`; see
  `docs/backlog/resolved/xwayland-support-done.md`'s Phase 4 record):
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

  Two more from the second review round (probe `probe6.sh` G and H, before
  at scoot `cfe5525` on `53aafc36`, after at the fixed head;
  `~/evidence/xw4/review2/`):
  - `0d553527` **sweep on a timer while transfers are in flight**
    (`SWEEP_INTERVAL`, 1 s, dropped when none are). The sweep had run only
    on the next selection event, so an owner killed mid-transfer left the
    paste waiting 40 s+ until some unrelated copy; unanswered conversions
    never checked their reader. A transfer is now treated as orphaned by
    ownership count, so one still moving when its owner died ends about a
    second after it stalls.
  - `5b575329` **each owner client gets a share of the transfers under
    way** (4 of the 8): one owner trickling a byte per chunk held all 8, and
    a different owner's paste read nothing for 35 s+. An owner can name
    another client's window, which only moves whose share it spends; the
    total holds (the same caveat applies to the outgoing per-client count).

  One more outside the XWayland series, against the same base:
  - `74edbf32` **clamp the pixman source image to its edge when scaling**
    (`Repeat::Pad`). The backend paired `Filter::Bilinear` with
    `Repeat::None`, so a bilinear tap at the texture edge read past it,
    where pixman returns transparent black, and any upscaled surface faded
    to a semi-transparent 1-px border: a 1x1 XRGB buffer viewported to the
    window read alpha 0.25 at the corners and 0.5 at the edge midpoints, a
    scale-1 buffer on a scale-2 output blended its 1-px edge at about 0.75
    (corners 0.56) over the background. The GLES backend samples with
    `CLAMP_TO_EDGE` and never had it. Pinned by
    `compositor/pixman_upscale/tests.rs` (fail-first on the pre-fix fork
    under pixman; the scale-2 test passes under `SCOOT_TEST_RENDERER=gles`
    on both revs), plus a downscale test that is exact on both revs, pinning
    the fix changed nothing there. See
    `docs/backlog/resolved/shm-viewport-upscale-edge-fade-done.md`.

  One more on top, from the XWayland pointer focus's X arm:
  - `6e6fe896` **flush the XDND proxy's remap when a drag leaves an X
    window.** `X11Surface`'s `DndFocus::leave` maps the window manager's
    full-screen XDND proxy back (and raises it) when an X-origin drag
    leaves an X window, but never flushed the connection -- unlike the unmap
    in `enter` -- so the requests sat in x11rb's write buffer until some
    unrelated request flushed it. Every X drag starts over its own window,
    so with scoot's pointer focus routing X-origin drags to the X target
    (needed for X to X and in-window moves), an X drag then found no window
    over a Wayland one and could not drop there. Measured fail-first with
    that routing on, at `74edbf32`, 3 of 3 runs:
    `xwayland/tests/drop.rs`'s
    `an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland` and
    `an_x_drag_whose_hovered_window_closes_finds_the_proxy_again` failed
    `left: "no window", right: "Smithay XDND proxy"`; at `6e6fe896` all
    drop and drag-gate tests pass, 3 of 3. See
    `docs/backlog/resolved/xwayland-pointer-focus-x11-done.md`.

  Two more from the review of that X arm, which made Smithay's X drop
  target reachable from a Wayland drag (scoot never made an X offer
  before). Acceptance tests: `compositor/xwayland/tests/drop_end.rs`, six
  tests, all failing at `6e6fe896`:
  - `7388af13` **tell a Wayland source about a drop onto X once, and only
    if it happened** (review N1). `DnDGrab::drop` already tells the source;
    `X11Surface`'s `DndFocus::drop` (and `handle_status`, for a deferred
    drop) told it `drop_performed` again, so a validated drop sent
    `wl_data_source.dnd_drop_performed` twice, and a refused one sent
    `dnd_drop_performed` then `cancelled` and still sent `XdndDrop`. An
    unaccepted offer is now left (`XdndLeave`); a deferred drop whose answer
    refuses is left and cancelled. Measured at `6e6fe896`: the finished
    control heard `["dnd_drop_performed", "dnd_drop_performed",
    "dnd_finished"]`; a refusing target was sent `XdndDrop`.
  - `9515d7e5` **end an XDND offer whose target is gone or never
    finishes** (review B1, blocking). The offer lived until `XdndFinished`,
    and while it lived `xfixes_selection_notify` took `XdndSelection` back
    from any X client: a target destroyed or hung after the drop stopped
    every later X drag, X to Wayland included, until scoot restarted. The
    offer now ends when its target or proxy is destroyed (a dropped one
    cancels its source), and a dropped offer is given up when an X client
    takes the selection (narrowed by `d3a4cd73` to a drag `allow_drag`
    accepts); one still in flight keeps it. Measured at
    `6e6fe896`: "an X drag could not start" after a silent target died and
    after a target hung past the drop. With `7388af13` alone, 4 of the 6
    tests pass (dying- and hanging-after-the-drop fail), so each commit is
    load-bearing; with both, all 6.
  - `d3a4cd73` **only a real X drag gives up a pending drop onto X**
    (final review N-A, N-B). `9515d7e5` gave up a dropped offer whose
    target had not finished whenever any X client took `XdndSelection`,
    before the grab lookup and `allow_drag`, so an X client with nothing
    held could end a drop in flight and keep the selection, and a target
    converting late would read that client's data. Now only a drag
    `allow_drag` accepts gives it up; any other taker is taken back from,
    and the take-back is flushed. It also cancels the source of a stale
    dropped offer that a new Wayland drag onto X replaces, which otherwise
    never heard how its drop ended. Acceptance test:
    `drop_end.rs`'s `a_rough_x_client_cannot_end_a_pending_drop`, which
    fails at `9515d7e5` (`left: 8388608 right: 8388608`, the rough
    client's window still owning the selection) and passes at `d3a4cd73`;
    the drop, drop_end, dnd, xdnd, clipboard and peer suites 46/46, 3 of 3
    runs; the full `--features xwayland` nextest run 1851 passed, 24
    skipped.
  - `b1ac3ca7` **an X drag enters another client's X window without
    waiting for types.** Unlike the others this touches Smithay's generic
    `input/dnd`, not only the XWM: a new `DndFocus::enter_needs_metadata`
    (default `true`, the old behavior) that `DnDGrab` asks before it
    delays entering a target for want of mime types, and that also lets it
    enter such a target before leaving the old one. `X11Surface` answers
    `false` for an X drag over another client's X window (or the source's
    own, once it has named types), and its `leave` skips the proxy remap
    when another X window was entered since. scoot forwards the method in
    `compositor/pointer_focus.rs`. GTK and Qt name their types to nobody
    when an X drag starts (before the proxy exists), so the grab entered
    nothing, the proxy stayed over the target, and a drop released on the
    first motion onto another X app went to the proxy; and a drag between
    two X windows mapped the proxy back for a moment, which GTK's window
    cache could see. Measured live with GTK `mousepad` over X: a drag
    reaching the other window in one motion landed 0/20 at `d3a4cd73`,
    20/20 at `b1ac3ca7`; five steps in the source then the jump 18/20 ->
    20/20; X to Wayland unchanged, 10/10. Acceptance tests:
    `compositor/xwayland/tests/first_motion.rs`'s three pins, which fail on
    every run at `d3a4cd73` (with only the metadata half, the crossing pin
    still fails) and pass at `b1ac3ca7`; the first_motion, drop, drop_end,
    dnd, xdnd, clipboard and peer suites 49/49, three runs; the full
    `--features xwayland` nextest run 1870 passed, 25 skipped; the
    workspace 2357 passed, 25 skipped (against a local path patch of this
    exact commit, before the repin). See
    `docs/backlog/resolved/xwayland-x-drag-first-motion-race-done.md`.
  - `7e18b661` **remap the proxy only once the drag has left every X
    window.** Found by the independent review of `b1ac3ca7`: `leave`
    skipped the remap only when the window last entered was a *different*
    X window, but a window mapped again under the drag is the same window
    id under a new `wl_surface`, so the grab entered it, then left the old
    surface, which saw its own window as the one hovered and mapped the
    proxy back over it. `XwmSourceState` now counts X windows entered and
    not yet left, and `leave` remaps only when that drops to 0; its trace
    line now says whether it remapped. Reproduced in scoot with a managed
    X window unmapped and mapped again under an X drag
    (`first_motion.rs`'s `an_x_drag_over_a_window_that_remaps_keeps_the_proxy_away`):
    fails 3 of 3 at `b1ac3ca7` ("found \"Smithay XDND proxy\" under the
    pointer"), passes 3 of 3 at `7e18b661`. (An override-redirect window
    re-derives the pointer focus as it unmaps, so it never takes that
    order.) The first_motion, drop, drop_end, dnd, xdnd, clipboard and peer
    suites 50/50, three runs; the full `--features xwayland` nextest run
    1885 passed, 25 skipped.
  - `b16cd6a2` **wait for types only over the window the drag started
    on.** `b1ac3ca7` waited over every window of the drag owner's X
    client, and single-instance apps (mousepad by default, GApplication
    apps generally) run every window on one X connection, so a quick drag
    between two of their windows found the proxy over the second on its
    first motion and dropped nothing (review of `b1ac3ca7`, live: 0/5; a
    few motions first, 4/4). `XwmActiveDrag` now records the X window the
    press or touch started on (from the grab's start focus) and the drag
    waits only over it; with no X window there, the owner's whole client
    waits, as before. Pinned by `first_motion.rs`'s
    `an_x_drags_first_motion_onto_its_own_clients_other_window_finds_that_window`:
    fails 3 of 3 at `7e18b661` ("found \"Smithay XDND proxy\""), passes
    3 of 3 at `b16cd6a2`; the first_motion, drop, drop_end, dnd, xdnd,
    clipboard and peer suites 51/51, three runs (X to Wayland drags,
    which start over their own window, included). See
    `docs/backlog/resolved/xwayland-same-client-quick-drag-done.md`.
  - `fcf6f314` **(on the branch, not pinned)** adds a public
    `X11Surface::set_commits_allowed`, so the window manager can withhold
    `_XWAYLAND_ALLOW_COMMITS` from an X window it refuses. It was written
    for `docs/backlog/protocols/xwayland-refused-windows-still-commit.md`
    and measured in scoot against a local repin before anything was
    pinned. It does not help: XWayland spends a refused override-redirect
    window's two buffers before the window manager hears of the map (480
    buffers either way for 240 menus, 128 drawn). scoot does not call it. Nothing uses it, so drop it at the next rebase
    unless a use turns up.
  - `e7130254` **apply a commit's buffer scale and transform without a new
    buffer.** `wl_surface.set_buffer_scale` and `set_buffer_transform` are
    double-buffered state that applies on the next commit with or without a
    new `attach`, but `RendererSurfaceState::update_buffer` read the cached
    values only in the `BufferAssignment::NewBuffer` arm, so a scale-only
    commit kept rendering at the old scale (measured: a 1600x1000 buffer at
    scale 1, committed at scale 2 with no new attach, drew its top-left
    quarter over the whole surface until a re-attach). The cached values
    are now read on every commit that has a buffer; the surface view is
    already recomputed each commit, so it follows. Content-identical to
    `6ab8b4a2` (the same fix cherry-picked onto `7e18b661` on
    `scoot/buffer-scale-without-new-buffer`, which `main` pinned first;
    it touches only `src/backend/renderer/utils/wayland.rs`). Pinned
    in scoot by
    `compositor/output_scale/tests.rs`'s
    `a_scale_only_commit_rescales_the_surface` (fails on the pre-fix fork,
    passes after), plus the scale-back-to-1 leg covering several scales in
    a row. See
    `docs/backlog/resolved/buffer-scale-without-new-buffer-done.md`.
  - `035d447c` **flush XSETTINGS writes.** `XSettings::update` wrote
    `_XSETTINGS_SETTINGS` and never flushed. The window manager's event
    thread only reads, so the write sat in x11rb's buffer until some
    unrelated request flushed the connection, and a toolkit reading the
    settings in between (one started right after XWayland was ready, or
    any X client after a settings change while no X window needed a
    configure) saw none of them. In scoot an X app at output scale 2 read
    no `Gdk/WindowScalingFactor` and drew at half size:
    `compositor/xwayland/tests/scale.rs` failed 2 runs in 20 on
    `e7130254` and passed 20 in 20 against a path patch of exactly
    `035d447c` (measured by the coordinating session), then 20 in 20
    again with the rev pinned from git (`-E
    'test(/xwayland::tests::scale::/)'`, 12 tests a run, 2026-09-28).
    `update` now flushes, which covers `set_xsettings`,
    `remove_xsettings` and `clear_xsettings`; it touches only
    `src/xwayland/xwm/settings.rs`.
  - `7ab72d53` **scan out a compositor-owned dma-buf element**
    (branch `scoot/cursor-dmabuf-storage`, on `035d447c`).
    `UnderlyingStorage` gains `Dmabuf(&Dmabuf)`, and the DRM compositor
    takes it down the dma-buf `wl_buffer` path: `ExportBuffer::Dmabuf`
    (the GBM exporter imports it under the same explicit-modifier rule; the
    dumb exporter refuses it), the element framebuffer cache keyed by its
    `WeakDmabuf`, the scan-out buffer kept alive while on screen, the
    y-invert flag, and the cursor-plane paths (no copy fast path; pixman
    maps it). Until now only a client's buffer could reach a plane:
    `Memory` storage has no export (`drm/exporter/mod.rs` and
    `ScanoutBuffer`/`ElementFramebufferCacheBuffer::from_underlying_storage`
    in `drm/compositor/mod.rs` all map it to `None` at `035d447c`), so
    scoot's drawn cursor, offered an overlay on a CRTC with no cursor plane,
    always composited, and a composited cursor denies the window under it
    the primary plane (Smithay tries the primary only for the bottom
    element with nothing rendered above it, `drm/compositor/mod.rs` ~1995).
    Measured on the Apple M2 (`Asahi.md`, Test 17): fullscreen mpv with the
    pointer visible and idle, 29-32 jiffies per 10 s composited before, 13-15
    with the cursor on overlay plane 45 and mpv's buffer on the primary after;
    pointer motion over a plain desktop, 19-20 against 9-10. Used by
    `compositor/render/cursor_plane.rs` (scoot does not compile against a
    rev without the variant). Touches `renderer/element/mod.rs`,
    `drm/exporter/{mod,gbm,dumb}.rs` and `drm/compositor/mod.rs`.
    **Alternatives considered**, each checked against `035d447c`:
    - *A scoot-side render element wrapping the cursor in a client-shaped
      `wl_buffer`.* `UnderlyingStorage::Wayland` takes a
      `renderer::utils::Buffer`, which `Buffer::with_implicit` builds from
      any `WlBuffer`, and `get_dmabuf` reads only the buffer's `Dmabuf`
      user data (`wayland/dmabuf/mod.rs:1025`), so scoot could mint one: a
      `WlBuffer` with `Dmabuf` data created for an in-process client on a
      socketpair it never reads. Rejected: it works by inventing a Wayland
      connection the scanout path then depends on. The object would be a
      server-created `wl_buffer` its client never asked for;
      `InnerBuffer::drop` sends `wl_buffer.release` on it
      (`renderer/utils/wayland.rs:68-79`) into a socket nobody reads, so
      scoot would need a drain as well; and the cursor's plane would hang on
      a client entry that any disconnect or kill path could end. scoot's
      per-client bounds would mostly leave it alone (they fire on requests
      it never sends, and `wl_buffers::forget_buffer` saturates), but each
      later one would have to be checked against it.
    - *A scoot-side exporter wrapper* (scoot already wraps the exporter,
      `tty/layout_exporter.rs`). Rejected: `Memory` storage never reaches
      the exporter. `element_config` returns `Unsupported` from
      `ExportBuffer::from_underlying_storage` first, and `ScanoutBuffer`
      and the framebuffer cache key are private to `drm/compositor` and
      refuse it too.
    - *An in-tree copy of `DrmCompositor`* (4,400 lines). Rejected for its
      size and for forking all plane assignment to change three
      `from_underlying_storage` arms.
    - *Hiding the pointer instead* (`[appearance] cursor_hide_after_ms`,
      shipped). Kept, and still the answer where no overlay is free, but it
      only helps once the pointer has sat still, and is opt-in.
- **Evidence:** `docs/backlog/resolved/syncobj-handle-leak-done.md`, and on the
  dev VM `~/evidence/sync/master-validation/`. Upstream master `79bbed5e1`
  (2026-09-22) was built and measured: it leaks 3.5–4.1 MB per test run,
  against about zero with the fork.
- **Upstream status (last checked 2026-09-25, master `79bbed5e1`, plus the
  pixman hunk re-checked 2026-09-27 and the XDND remap flush 2026-09-27 at
  master `928d4a9b`):** all eighteen earlier ones unfixed on master (the XWM code carries
  the double delete, the `O_NONBLOCK` pipe, the unpaused read, the
  unflushed owner change, the unflushed proxy remap, the doubled
  `drop_performed`, the offer that outlives its target and no hooks; the
  pixman scaler still pairs `Filter::Bilinear` with `Repeat::None` at the
  same site). `d3a4cd73` narrows `9515d7e5`'s give-up, which exists
  only in the fork, so it has no upstream counterpart of its own.
  `b1ac3ca7` (checked 2026-09-27, master `928d4a9b`): upstream's
  `DnDGrab` still delays every enter until the source has mime types
  ("delay until they have materialized"), and has no
  `enter_needs_metadata`; `7e18b661` fixes fork-only code from
  `b1ac3ca7`, and `b16cd6a2` narrows it further. `e7130254` (as
  `6ab8b4a2`, checked 2026-09-27, current master): upstream's
  `update_buffer` still reads `buffer_scale`/`buffer_transform` only in
  the `NewBuffer` arm. `035d447c` (checked 2026-09-28, master
  `928d4a9b`): upstream's `XSettings::update` still returns after
  `change_property8` without a flush. `7ab72d53` (checked 2026-10-06,
  master `19c16d3e`): upstream's `UnderlyingStorage` and `ExportBuffer`
  still have no dma-buf variant. No issue
  or PR exists. Nothing has been filed from here.
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
  its resolved record; `compositor/pixman_upscale/tests.rs` fails on the
  pre-fix fork for both upscale cases (the downscale test passes on both
  revs, pinning the no-change half); `compositor/xwayland/tests/drop.rs`
  fails without `6e6fe896` (two tests, "no window" where the proxy should
  be); `drop_end.rs` fails without `7388af13`, `9515d7e5` and `d3a4cd73`;
  `first_motion.rs` fails without `b1ac3ca7` (its remap test without
  `7e18b661`, its same-client test without `b16cd6a2`), and scoot does not
  compile against a rev without `DndFocus::enter_needs_metadata`;
  `compositor/output_scale/tests.rs`'s `a_scale_only_commit_rescales_the_surface`
  fails without `e7130254` (the far pixel is still window fill);
  `compositor/xwayland/tests/scale.rs` fails intermittently without
  `035d447c` (2 runs in 20: the X app reads no scale); scoot does not
  compile against a rev without `UnderlyingStorage::Dmabuf` (`7ab72d53`).

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
