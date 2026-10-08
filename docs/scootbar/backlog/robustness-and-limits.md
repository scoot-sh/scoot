---
title: "Robustness and resource limits: hostile input, failing children, a dying compositor"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "ongoing"
---

# Robustness and resource limits

Filed 2026-09-29. Serves **daily-drive**. The release profile is
`panic = "abort"`, so a panic kills the bar and its content. `CLAUDE.md`'s
bar applies: a plausible crash or hang is treated like data loss. Modeled on
what scootbg and scoot bound (`docs/scootbg/README.md`, `docs/ipc.md#resource-bounds`).

## Bounds to build in from the first PR that adds the surface

- **Control socket** (*built*: 16 connections with the oldest
  non-subscriber evicted to admit one more (#339); 4 subscribers with the
  5th refused with a reason (#367); 64 KiB request lines refused with a
  reason (#339); the EMFILE spare fd plus a 1 s listener rest instead of
  a spin (#339); the write-stall deadline landed in #493, 30 s
  for a request peer that stops reading, refreshed on any delivery):
  connection cap with a refusal that says why; line and message size caps;
  a write-stall deadline that drops a peer that stopped reading; `EMFILE`
  on accept sheds instead of spinning the loop.
- **`exec` modules** (*built*, with
  [exec-push-button-modules](resolved/exec-push-button-modules-done.md): at most
  8 placed, lines past 4096 bytes dropped whole, one 4 KiB read per 16 ms frame
  while output keeps coming, restarts 1 s to 60 s, a pidfd to reap, the process
  group killed with the module, a fuzz target for the payload, and a flooding
  child against a running bar with the RSS and fd count checked in
  `tests/exec.rs`): a cap on the number of children and on line length (drop
  the excess with a warning); update rate coalesced to the next frame; restart
  with exponential backoff; children reaped (no zombies), never inheriting the
  bar's fds (close-on-exec everywhere, verified as scoot's fd audit did); a
  child that never prints is fine, one that floods cannot grow memory.
- **Config and JSON** (*built*: the file is capped at 64 KiB, read with
  `.take(MAX_FILE + 1)` from a regular file opened non-blocking (#339,
  #513); update payloads are capped at 4096 bytes and 8 levels deep
  (#366); every parse failure is a named error, never a panic, and hot
  reload keeps the running config on failure (#339).
- **Text**: bounded glyph cache and bounded string lengths per module.
  *Built with the clock
  ([module-api-and-clock](resolved/module-api-and-clock-done.md)):* the
  glyph cache holds at most 512 glyphs and 4 MiB of coverage and is dropped
  and refilled past either; a glyph whose bounds would need a rasterizer
  buffer past 4 megapixels (a hostile font) is not drawn; a module's view
  text and tooltip are cut at 256 bytes; control characters are never
  drawn. **The font mapping is decided there**: a font is mapped only when
  it is owned by root, writable by no one and on a read-only mount (a
  read-only mount alone is not enough, as review showed), and read into
  the heap everywhere else, at most 64 MiB, so a `cp` over the font under a
  running bar cannot `SIGBUS` it.
  What is left here is a title stream fuzzed through the cache
  ([icons-and-fonts](resolved/icons-and-fonts-done.md) owns that test).
- **Time zone file**: the TZif reader caps the file (64 KiB in the spike),
  checks every count and index, and falls back to the last transition or to
  UTC rather than failing ([M0 §3](resolved/dependencies-done.md#3-time-zone)).
  *Built with the clock:* the cap is enforced while reading (at most 64 KiB
  + 1 bytes, from a regular file only, opened without blocking, so
  `TZ=:/dev/zero` or a FIFO cannot hang or balloon it), the instant is
  clamped before any arithmetic, and a `cargo fuzz` target covers the
  reader and the POSIX rule parser.
- **Module count and layout** (*built*: at most 32 modules placed (#324);
  a layout wider than the output clips deliberately, not by overflow:
  `layout.rs` uses saturating arithmetic on every client-controlled size
  (#324)).

## Where other bars actually break

From the research (2026-09-29; the sources are GitHub issues, and the report
notes it could not reach Reddit or Hacker News), the failures that recur are
**not** one-frame costs but lifecycle ones. Each gets a named test:

- **Suspend/resume and DPMS wake**: infinite loops and 100% CPU after resume,
  segfaults on wake (Waybar #4393, #1019). Test: suspend cycle with a dead fd in
  the poll set; assert idle CPU returns to zero.
- **Output hotplug**: crashes on HDMI disconnect, modules that stop updating
  after hotplug (Waybar #2808, #4823). Test: repeated add/remove with the module
  set running; assert every module still updates.
- **Memory growth over days**: leaks with only two modules enabled (Waybar #5186),
  runaway growth in a privacy module (#3981). Test: a multi-day soak in the
  [ratchet](lightest.md), and no per-event allocation on the hot path.
- **Busy loops from polled scripts** (Waybar #5303, #4987): see the streaming
  rule in [exec-push-button-modules](resolved/exec-push-button-modules-done.md).
- **Tray failures** (the largest single group): see [tray](resolved/tray-done.md).
- **Zero outputs** and a compositor restart mid-session.

## Failure of the world around it

- **Compositor gone**: the Wayland connection breaks; exit promptly and cleanly.
- **Registry changes** (*built*: the `ext-workspace` and toplevel managers'
  `finished` drops staged state and destroys the handles, with tests
  (#334, #374); an output or global removed mid-frame is handled by the
  output set; a layer surface `closed` by the compositor is rebuilt once
  the output is still there, then given up on for that output's life
  (#323); a `wl_output.name` past 128 bytes is ignored (#539).
- **Allocation failure**: know where the bar can abort on refused memory (the
  scaler in scootbg is the precedent,
  [`scaler-oom-abort`](../../scootbg/backlog/resolved/scaler-oom-abort-done.md)); avoid large
  allocations sized by external input.
- **Restart policy** (*built*: the NixOS and home-manager units set
  `Restart=on-failure` with `RestartSec=2`, bound to
  `scoot-session.target` (#454); scoot itself does not supervise clients).
  What is left is the docs half of
  [nix-modules-and-stylix](resolved/nix-modules-and-stylix-done.md): say in the
  docs what happens without the unit.

## Verification

Fuzz (see [testing-and-ci](resolved/testing-and-ci-done.md)), an fd storm and a flooding
`exec` child against a running bar with the RSS and fd count measured before
and after, and a kill of the compositor mid-frame.

## Done when

Each bound above has a test that fails without it, and none of the storms moves
RSS or fd count beyond a stated margin.

## Audit 2026-10-08

The ticket is claimed live by `rob2` (claimed 2026-10-07, still within TTL
when checked; the claim was left alone, not forced). The external-input
vectors the brief named were audited against the code instead of re-bounded,
and all are already capped, each with a test that fails without it:

- **D-Bus**: messages past 1 MiB are skipped whole (`MAX_MESSAGE`,
  `frame_header`), the spec's own 128 MiB ends the connection (`MAX_WIRE`);
  nesting past 32 refused (`MAX_DEPTH`); names/paths/signatures at the
  spec's 255/1024/255 (`MAX_NAME`, `MAX_PATH`, `MAX_SIGNATURE`); strings are
  borrowed and cut where they are used (tray titles at 128 bytes, MPRIS
  fields at 120, view text at 256). All in #388 and its follow-ups.
- **Tray pixmaps and menus**: 64 pixmaps of at most 256 px a side
  (`MAX_PIXMAPS`, `MAX_PIXMAP_SIDE`), 64 menu items at 8 levels
  (`MAX_MENU_ITEMS`, `MAX_MENU_DEPTH`), 128 properties (`MAX_PROPERTIES`),
  32 tray items (`MAX_ITEMS`), all refused whole with the last state kept.
- **Config sizes**: 64 KiB file cap above; icon files 8 MiB with 1 Mpx and
  16 MiB decode budgets; fonts 64 MiB.
- **Protocol vectors**: 32 configured outputs (policy; live `wl_output` globals follow the compositor and are not counted), 8 ext-workspace groups with 32
  workspaces (#334), 64 toplevels with 64-byte app ids (#374), 128-byte
  output names (#539).

Two walks deliberately stop at the message cap plus a consumer cap rather
than a walk cap, and both say so in their module docs with tests: MPRIS
artists (walks the list, keeps 16) and BlueZ managed objects (bounded by
the 1 MiB message, held to 8 adapters and 64 devices). No new bound was
invented on top of those decisions.

Genuinely open: the suspend/resume DPMS test, the hotplug storm test, the
multi-day soak, compositor-gone mid-frame, zero outputs with a compositor
restart, the allocation-failure audit, and the restart-policy docs half
above. This entry stays `open` (`ongoing`): it is the checklist, not a task
to resolve.
