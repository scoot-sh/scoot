---
title: "Restore windows, workspaces and binds when a monitor reconnects"
status: "open"
area: "core"
priority: "high"
blocked: "none — design below; needs the --tty hotplug of milestone 19 phase E (landed)"
---

# Restore windows, workspaces and binds when a monitor reconnects

Filed 2026-09-25 from PR #247's review, after a physical DP-1 replug on the
Asahi M2 Air (`Asahi.md` Test 3): the monitor came back as a *new* output
(id 3, not 2) with an empty workspace, while both windows stayed piled on
the panel.

## Why it matters (daily-drive)

DisplayPort monitors commonly drop hot-plug detect when they enter standby
and re-assert it on wake. Under `--tty` every such cycle is now an unplug and
a replug (milestone 19 phase E). The result:

- **Windows pile up.** Every window on the external screen is adopted by the
  remaining output and stays there, so after a screen-saver cycle the laptop
  panel holds everything.
- **Binds stop working.** The monitor returns under a fresh `OutputId`, so
  the default `Super+period` / `Super+Shift+period` binds (output id 2) stop
  reaching it until scoot restarts.

## Shape (niri-style)

- **Identity.** Match a returning output by connector identity, not by
  `OutputId`:
  - the connector name (`DP-1`), plus the EDID make/model/serial where the
    connector has an EDID blob;
  - name alone as the fallback, since a panel has no serial and a KVM may
    hide the EDID.
- **Remember what left.** When `State::remove_output` removes an output,
  record which workspaces (and their windows, in order) the core migrated
  off it, keyed by that identity.
- **Give it back.** When an output with a matching identity is added, move
  the still-open ones back to it: the same workspaces, active index and
  column order.
  - A window the user has since moved elsewhere by hand stays where it is.
  - Closed windows drop out.
  - Needs a `scoot-core` action or event to adopt workspaces back onto an
    output. The core's `remove_output` already moves them in one piece, so
    the reverse is its mirror. Keep it platform-independent.
- **Binds follow the connector.** The default output binds should name "the
  first/second screen" rather than a raw id: either reuse the returning
  connector's previous `OutputId`, or resolve binds by position/identity at
  dispatch time. Pick one and document the stability rule in
  `docs/configuration.md#moving-across-outputs`.
  - This subsumes the "Output ids on replug" follow-up in
    [multi-output-remainder](./multi-output-remainder.md).
- **Debounce (worth measuring first).** A monitor that flaps HPD rapidly
  would remove and re-add its output each time. A short grace period before
  removal (niri waits for the connector to settle) may be worth it once
  restore exists. Record the measured flap timing from real hardware before
  choosing a number.

## Pins it will need

- **Harness:** remove output 2 holding two workspaces, add an output with
  the same identity, and check that the windows and workspaces are back in
  order and the active index is restored.
- **Harness:** add an output with a *different* identity, and check that
  nothing moves.
- **Harness:** a window moved by hand in between, and check that it stays.
- **Live:** a DP replug on the Asahi M2 Air
  (`~/fx/replug-start.sh`/`replug-collect.sh`), including the default
  output-2 bind reaching the returned monitor.
