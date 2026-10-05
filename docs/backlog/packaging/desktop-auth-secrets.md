---
title: "Desktop auth: polkit agent and secrets/keyring"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
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
