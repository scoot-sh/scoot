---
title: "Robustness and resource limits: hostile input, failing children, a dying compositor"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "skeleton-layer-surface"
milestone: "ongoing"
---

# Robustness and resource limits

Filed 2026-09-29. Serves **daily-drive**. The release profile is
`panic = "abort"`, so a panic kills the bar and its content. `CLAUDE.md`'s
bar applies: a plausible crash or hang is treated like data loss. Modeled on
what scootbg and scoot bound (`docs/scootbg/README.md`, `docs/ipc.md#resource-bounds`).

## Bounds to build in from the first PR that adds the surface

- **Control socket**: connection cap with a refusal that says why; line and
  message size caps; a write-stall deadline that drops a peer that stopped
  reading; `EMFILE` on accept sheds instead of spinning the loop.
- **`exec` modules**: a cap on the number of children and on line length (drop
  the excess with a warning); update rate coalesced to the next frame; restart
  with exponential backoff; children reaped (no zombies), never inheriting the
  bar's fds (close-on-exec everywhere, verified as scoot's fd audit did); a
  child that never prints is fine, one that floods cannot grow memory.
- **Config and JSON**: size caps, depth limits, and every parse failure a
  named error, never a panic. Hot reload keeps the running config on failure.
- **Text**: bounded glyph cache and bounded string lengths per module. The
  font file is mapped ([M0's choice](resolved/dependencies-done.md#1d-mapped-or-read)),
  so a font truncated in place under the running bar is a `SIGBUS`: decide
  here whether to document it or read files outside `/nix/store` into the heap.
- **Time zone file**: the TZif reader caps the file (64 KiB in the spike),
  checks every count and index, and falls back to the last transition or to
  UTC rather than failing ([M0 §3](resolved/dependencies-done.md#3-time-zone)).
- **Module count and layout**: a configured list has a maximum; a layout wider
  than the output clips deliberately, not by overflow (saturating arithmetic on
  every client-controlled size).

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
  rule in [exec-push-button-modules](exec-push-button-modules.md).
- **Tray failures** (the largest single group): see [tray](tray.md).
- **Zero outputs** and a compositor restart mid-session.

## Failure of the world around it

- **Compositor gone**: the Wayland connection breaks; exit promptly and cleanly.
- **Registry changes**: the `ext-workspace` manager sending `finished`, an
  output or global removed mid-frame, a layer surface `closed` by the
  compositor (rebuild if the output is still there).
- **Allocation failure**: know where the bar can abort on refused memory (the
  scaler in scootbg is the precedent,
  [`scaler-oom-abort`](../../scootbg/backlog/scaler-oom-abort.md)); avoid large
  allocations sized by external input.
- **Restart policy**: scoot does not supervise clients. Ship a systemd user
  unit / home-manager `Restart=` in [nix-modules-and-stylix](nix-modules-and-stylix.md), and
  say in the docs what happens without one.

## Verification

Fuzz (see [testing-and-ci](testing-and-ci.md)), an fd storm and a flooding
`exec` child against a running bar with the RSS and fd count measured before
and after, and a kill of the compositor mid-frame.

## Done when

Each bound above has a test that fails without it, and none of the storms moves
RSS or fd count beyond a stated margin.
