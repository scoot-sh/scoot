---
title: Displays
description: "Scale and power that follow the connected set when you dock and undock."
---

A laptop moving between a desk monitor and no monitor keeps its
scale and power following the connected set: dock, and the external
panel comes up at its own scale (and, with `disabled`, the lid panel
goes dark); undock, and the laptop panel takes over again, lit even
if the docked profile turned it off:

```nix title="home.nix"
programs.scoot.desktop.displays.profiles = [
  {
    name = "docked";
    outputs = [ "eDP-1" "DP-1" ];
    scale = {
      "DP-1" = 1.0;
      "eDP-1" = 2.0;
    };
  }
  {
    name = "undocked";
    outputs = [ "eDP-1" ];
    scale = { "eDP-1" = 2.0; };
  }
];
```

What a profile does is narrow: **scale and power, at hotplug.** It
names the exact connected set it answers (connector names, as
`scoot msg outputs` lists them: `eDP-1`, `DP-1` on real hardware) and,
per output, a scale and whether the panel stays off. It does not place
outputs (they pack left to right in connection order), does not set
modes, and cannot match on make or model (scoot's IPC reports
connector names only). The watcher (`scoot-displays`, a user unit
bound to `scoot-session.target`) re-matches once at session start
and on every output event: each plug (`output_added`, which fires for
a monitor's first plug and every replug), each unplug, and each mode
change, settling 2 s after the first of a burst. First match wins, in
list order. With the [idle policy](./idle-lock.md) on, it also
re-applies when the screens come back from idle.

How a match applies, all over scoot's own IPC, with no config file
written:

- **Scale.** Each output of the set runs at the profile's scale
  (`scoot msg output-scale NAME SCALE`). An output the profile gives
  no scale goes back to your config's scale
  (`scoot msg output-scale NAME reset`), so one profile's scale never
  lingers into the next.
- **Power off.** Each output the profile's `disabled` names is
  powered off (`scoot msg output-power ID off`), and the watcher
  records that it did.
- **Power back on.** An output the watcher itself turned off comes
  back on as soon as the matched profile stops disabling it, or no
  profile matches at all. Undocking from a clamshell
  profile: `docked` darkens `eDP-1`, the monitor leaves, and `eDP-1`
  lights again whether or not an `undocked` profile exists.
- **Every other power state is left alone.** An output the watcher
  did not turn off is never turned on: screens the idle policy
  blanked stay dark through a monitor's standby unplug, and so does
  one you turned off by hand.
- **No match.** Every connected output goes back to your config's
  scale, and the watcher's own offs come back on.

The record of its own offs is `$XDG_RUNTIME_DIR/scoot-displays.off`.
It goes when your last session logs out (unless lingering is on), but
it survives a compositor restart inside one login,
so a fresh scoot can find an old record:
until the watcher's first apply in that session, an entry whose name
and id match a live output can turn that output on once. It only ever
lights a screen, never darkens one. Everything else is live runtime
state, not config: your config file (under home-manager, a read-only
link into the Nix store) is never touched. A successful
`scoot msg reload` or a restart goes back to the config's scales until
the watcher applies again (at the next output event, idle resume or
login). A home-manager rebuild never reloads the session, so it never
drops an applied scale. The apply reports `applied` only when every
call succeeded. Otherwise it exits non-zero and names each failed
call, and the unit's journal shows the line.

Why a scoot-native watcher instead of kanshi: stock kanshi runs its
hooks only after the compositor answers an output configuration with
success, and scoot's output-management write half answers every
configuration with failure by design: kanshi matches the profile
through the read half and then fails it, every hotplug, applying
nothing. The watcher speaks scoot's own IPC instead (`subscribe` for
events, `outputs` for matching, `output-scale` and `output-power` for
applying), waking on hotplug the same event-driven way kanshi does:
blocking read, no polling, no idle wakeups. It is a stopgap. The
durable answer is a writable output-management protocol, which would
let kanshi (and positions and modes) work as-is; that work is tracked
in [the output-management reconfiguration entry](https://github.com/scoot-sh/scoot/tree/main/docs/backlog/resolved/output-management-reconfiguration-done.md).

```sh
scoot-displays status
scoot-displays apply
scoot msg outputs
systemctl --user status scoot-displays
```

Every value is an option, applied on rebuild/switch (new profiles
restart the watcher, which re-applies at start):

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.displays.enable` | bool | `true` with the profile | match the connected set and apply scale and power (with no profiles the watcher sets no scale and turns nothing off; it only turns back on an output it turned off itself under an earlier profile list) |
| `desktop.displays.profiles` | list of profiles | `[]` | arrangement profiles, first match wins in list order |
| `profiles[].name` | string (unique) | — | the profile's name, as `status` prints it |
| `profiles[].outputs` | list of strings (non-empty, unique set) | — | the exact connected connector-name set this profile answers |
| `profiles[].scale` | attrset of number (0.5–4.0) | `{ }` | per-output scale, applied live; an output in `outputs` with no entry runs at your config's scale (every key names an output in `outputs`) |
| `profiles[].disabled` | list of strings | `[ ]` | outputs to power off under this profile, live, and back on once it no longer applies (empty disables nothing; every entry names an output in `outputs`, and at least one output of `outputs` stays out of it) |

A mode is not a profile field: a mode cannot change live, and a
profile that sets `mode` fails evaluation. Set one per connector in
`programs.scoot.settings.outputs` (`{ name = "DP-1"; mode =
"3840x2160"; }`): it applies at session start, beside the watcher. A
static `scale` there is fine too. It is what an output runs at when no
profile scales it.

Troubleshooting, by symptom:

- *Nothing applies on plug.* Names first: `scoot msg outputs`
  lists the connected connectors, and the profile matches only the
  exact set. One extra monitor (or one missing letter) falls
  through. `scoot-displays status` prints the match (`none` plus
  the connected set) without changing anything.
- *The scale reverted after a reload.* A successful reload goes back
  to your config's scales, and no event tells the watcher about it.
  Run `scoot-displays apply` to converge now, or wait for the next
  plug.
- *A scale I set by hand reverted.* With profiles configured, every
  apply (each plug and unplug, and each idle resume) sets every
  connected output's scale: the profile's, or your config's when the
  profile gives none or nothing matches. A hand
  `scoot msg output-scale` lasts until the next apply; put the scale
  in a profile (or in `programs.scoot.settings.outputs`) instead.
- *`apply` exits non-zero.* Its stderr (and the unit's journal) names
  the call that failed and scoot's reason. An output unplugged
  mid-apply is the common one; the next output event applies again.
- *An output stays dark.* Either the matched profile disables it
  (`disabled`; `status` names the profile), or something other than
  the watcher turned it off (the idle policy, `wlopm`, a hand
  `output-power`), which the watcher deliberately leaves alone: the
  next input after idle lights it, or `scoot msg output-power <id> on`
  does (ids come from `scoot msg outputs`, stable for the session,
  fresh after every unplug). An off the watcher made itself always
  comes back on when its profile stops matching. Turning the slot
  off (`enable = false`) stops the watcher where it stands, so an
  output it had off stays off until that `output-power` call or the
  next login.
- *The laptop panel lit up while idle, docked.* A monitor that drops
  its connection in standby is an unplug to scoot, so with the
  docked profile no longer matching, the watcher lights the panel it
  darkened rather than risk leaving no screen on (it cannot tell that
  unplug from a real undock). The panel goes dark again when the
  monitor reconnects and the docked profile matches. The idle
  policy does not darken it again on its own: its screens-off fires
  once per idle stretch, so the panel stays lit until the monitor
  comes back or you return.
- *A disabled panel lit up after idle.* Without the idle policy,
  something else ran `wlopm --on '*'` (or `output-power all on`):
  `scoot-displays apply` turns it back off. With the idle policy on,
  its resume re-applies by itself.
- *Evaluation refuses a profile that "disables every output it
  matches".* Its `disabled` names every output of its own `outputs`,
  which would leave no screen on whenever it matched. Keep at least
  one out.
- *Two profiles could match.* The first in list order wins; two
  profiles for one set fail evaluation instead of shadowing.
- *The watcher is not running.* It starts with the session scope:
  `systemctl --user status scoot-displays` (and
  `scoot-session.target` beside it). With the slot on but no
  session yet, it retries.
