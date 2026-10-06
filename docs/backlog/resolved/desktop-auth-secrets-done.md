---
title: "Desktop auth: polkit agent and secrets/keyring"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Desktop auth: polkit agent and secrets/keyring

Filed 2026-10-04, child 6 of `desktop-paved-path`. Serves
**daily-drive** (without a polkit agent, every privileged GUI action fails
silently; without a keyring, every app re-prompts for secrets).

## The gap

Zero mentions of polkit, gnome-keyring, or any secrets provider in `nix/`,
`flake.nix`, `docs/nix.md`. A scoot desktop today has no
`org.freedesktop.PolicyKit1` authentication agent and no
`org.freedesktop.secrets` implementation.

## What to do

Fill the `desktop.auth` slot:

- Polkit agent (candidates: `lxqt-policykit-agent`, `hyprpolkit-agent`,
  `polkit-gnome` — pick the lightest well-maintained, record closure size,
  say why), started as a user unit in the session, so GUI privilege
  prompts (disks, network, printers) work.
- Secrets: `gnome-keyring` daemon with the `secrets` component (or
  `KeePassXC` (freedesktop-service — decide, say why) auto-unlocked from
  the login password where the greeter path allows (ReGreet/PAM shape —
  say what works and what degrades to a second prompt).
- Edge cases: headless/agent sessions with no agent (fail loud, not
  silent); SSH sessions sharing the user manager; the agent dying
  mid-prompt.

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2 (a
polkit prompt authenticates; a secret stored once is not re-prompted after
relogin); docs in `docs/nix.md`.

## Not in this ticket

Fingerprint/smartcard unlock; full-disk-encryption enrollment; managing
the user's PAM stack beyond what the greeter needs.

## Landed (PR #TBD, 2026-10-06)

Filled the `desktop.auth` / `desktop.secrets` slots, both on with the
profile (each still individually disable-able):

- Polkit agent: polkit-gnome 0.105, spawned in-scope by the session
  leader through the login entry's `SCOOT_POLKIT_AGENT` (supervised,
  restarts 2 s after death; entry without it is the plain launcher),
  with polkitd beside it. NOT a user unit, contrary to the ticket's
  sketch: proven live that a unit's agent stays connected but
  unregistered (polkit refuses with "User of caller and user of
  subject differs" -- a unit never joins the logind session, and this
  kernel has no audit, so the session scope is the only signal).
  Measured at the pinned rev on the M2 (full closure, marginal over
  the profile tools, idle RSS, wakeups over 60 s): polkit-gnome wins
  (335.9 MiB full, 403 KiB marginal, ~4.1 MB RSS, 0 wakeups) over
  lxqt-policykit 2.4.0 (858.9 MiB / 385.9 MB) and hyprpolkitagent
  0.1.3 (1.7 GiB / 642.9 MB); all three idle at zero wakeups, so the
  pick is closure. lxqt/hypr stay selectable through `daemon`.
- Secrets: gnome-keyring 50.0 D-Bus activated (`--components=secrets`,
  no unit) with `secret-tool` (libsecret) on PATH, over KeePassXC
  2.7.12 (412.6 MiB / 36.3 MB vs 542.4 / 177.7 MB; KeePassXC needs
  its GUI app running and has no PAM unlock path). PAM pair confined
  to greetd's own service (probed: nixpkgs' `enableGnomeKeyring`
  flag is a silent no-op there since greetd sets
  `useDefaultRules = false`); the stock gnome-keyring switch is not
  used (it owns the `login` service's PAM).
- Live proof (scoot-test greetd login, generation 100, restored to
  99 after): agent in `session-*.scope` registers (empty success
  reply, session-id correct); prompt appears and authorizes (dialog
  screenshots, dots in field, retained `tmpauthz` grants instant
  re-check exit 0; dummy password rejected with journal FAILED);
  SSH/agent-less sessions refuse loud ("no agent available"); agent
  kill restarts supervised (new PID re-registers and serves).
- Keyring boundary (honest degradation, acceptance partially unmet):
  PAM primes the unlock at auth (`gkr-pam: ... unlocked keyring` in
  the greeter log) but the PAM-started daemon dies with the greeter
  scope, so the session daemon starts locked and the first secrets
  use per login unlocks once (login password); CLI refuses loud
  until then, GUI apps show their unlock UI. A secret can therefore
  NOT be read with zero prompts after a logout+relogin on greetd --
  persisting the unlock would need the password to reach a surviving
  daemon, a security boundary not crossed here. All of the slot's
  machinery (PAM pair, activation, wrapper, client, bus names) is
  proven working.
- Docs on the site's desktop page (Privileges prompts and the
  keyring: measured pick tables, per-option Type/Default, symptom
  troubleshooting); launcher harness T16-T18 (spawn post-display,
  session-id scrub, supervised restart, missing-binary loud).

Battery: no polling anywhere (agent blocks on D-Bus, keyring starts
on demand); D-Bus activation for secrets, always-running (but
idle-silent) agent for polkit -- no activation protocol exists for
agents, stated.
