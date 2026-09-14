---
item: "2"
title: "Keybindings layer"
status: "done"
area: "input"
pr: 4
commit: "f3723b3"
---

# Keybindings layer

~~Keybindings layer~~ — DONE, merged to `main` at `f3723b3`, PR #4. Vim
motions (h/j/k/l) + Super, intercepted at `input::key`'s filter closure
(the single choke point shared by IPC/nested/future-tty input). Matches
unshifted level-0 keysym + tracked modifiers (not case-folded shifted
symbols) — deliberate deviation from initial spec, matches niri/sway
convention. Independent review found 2 issues, both fixed before merge:
(a) `press()` could leak a stuck modifier if the combo's main key resolved
by name but wasn't on the actual keymap — fixed by resolving the main
keycode before pressing any modifiers; (b) documented an invariant on
`suppressed_keys` (only `key()` may mutate it) against a desync risk
relevant once the tty backend's libinput device-teardown exists.
`crates/flexwm/src/compositor/keybindings.rs` holds the pure, unit-tested
lookup table — the seam a config-file loader parses `flexwm_ipc::KeyCombo`
strings into.
