---
item: "4a"
title: "Config file"
status: "done"
area: "config"
pr: 6
commit: "3b62292"
---

# Config file

Window decorations and a config file for "more visually pleasing options."
Split into two PRs. **4a config file — DONE, merged to `main` at
`3b62292`, PR #6.** `crates/flexwm/src/compositor/config.rs`: TOML at
`$XDG_CONFIG_HOME/flexwm/config.toml` (falls back to `~/.config/flexwm/`),
`--config PATH` flag; `[layout]` + `[binds]` sections. serde stays out of
`flexwm_core` (conversion by hand in the `flexwm` crate). Failure
semantics are the safety-critical part: explicit `--config` missing = hard
error; default path missing = silent defaults; malformed/typo'd config =
logs and falls back to full defaults, *never* blocks startup (a
compositor that won't boot over a typo is a lockout with no recovery on
real hardware). Bind collisions are detected and the whole group skipped
rather than an arbitrary "last wins." `--tty`'s Ctrl+Alt+Fn VT-switch
bindings always override a colliding config bind, applied after the
config loads. 51 tests, verified via `scripts/smoke-test.sh` under all
three backend modes including real `--tty` hardware.
