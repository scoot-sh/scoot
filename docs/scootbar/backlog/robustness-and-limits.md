---
title: "Robustness and resource limits: hostile input, failing children, a dying compositor"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "skeleton-layer-surface"
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
- **Text**: bounded glyph cache and bounded string lengths per module.
- **Module count and layout**: a configured list has a maximum; a layout wider
  than the output clips deliberately, not by overflow (saturating arithmetic on
  every client-controlled size).

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
