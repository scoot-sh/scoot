---
title: "home-manager: sessionScript is not written next to the config when configFile is overridden"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# home-manager: sessionScript is not written next to the config when configFile is overridden

Filed as gh issue #174 (read it — exact option/text, the missing test
pairing, both fix directions; this entry tracks it).

`nix/modules/home.nix`: `configFile` is settable but the session
script's path is hardcoded to `xdg.configFile."scoot/session.sh"`.
Both the module comment and `docs/nix.md` claim it is written "next to
the config" — set `configFile = "myscoot/custom.toml"` and the config
lands in `~/.config/myscoot/` while the script stays in
`~/.config/scoot/`. `nix/tests.nix`'s `hmRelocated` case never sets
`sessionScript`, so the pairing is untested.

Fix — either direction (decide + record why):
1. Make the claim true: `xdg.configFile."${dirOf cfg.configFile}/session.sh"`.
2. Or make the docs true: fixed `scoot/session.sh` independent of
   `configFile` (the option description already says to launch the script
   by explicit path, which favors this).

Either way extend `hmRelocated` to set `sessionScript` too, so the
pairing is pinned. Verify with `nix flake check` (the content checks run
the relocation case).
