---
title: "`flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/ `Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a real modifier off its `_L` key breaks `msg key` combos entirely (LOW, pre-existing)."
status: "open"
area: "input"
priority: "low"
blocked: null
---

# `flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/ `Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a real modifier off its `_L` key breaks `msg key` combos entirely (LOW, pre-existing).

`flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/
`Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a
real modifier off its `_L` key breaks `msg key` combos entirely (LOW,
pre-existing). Found by `flexwm-reviewer` while re-verifying item 14's
shifted-character fix (that PR's own `ModifierKeys::probe` — which asks
the keymap which key *actually* holds a given real modifier on the active
layout/group, rather than assuming a fixed keysym — sits one function away
from this bug and already knows how to answer it correctly). `input.rs`'s
`resolve_combo` maps `Modifier::{Ctrl,Shift,Alt,Super}` to a hard-coded
`_L` keysym and then demands it be reachable at level 0.

Concrete failure: `XKB_DEFAULT_LAYOUT=us,de XKB_DEFAULT_OPTIONS=grp:lshift_toggle`
(or `grp:lctrl_toggle`) is a real xkeyboard-config option that removes
`Shift_L`/`Control_L` from the keymap entirely, leaving `Shift_R`/
`Control_R` as the only key carrying that real modifier. `flexwm msg type
"A"` still works (the probe finds `Shift_R`), but `flexwm msg key
shift+a` and `flexwm msg key ctrl+c` are refused with ``no key for `shift`
in this layout`` — an agent on such a session cannot send Ctrl+C, or any
other modifier combo, at all. Verified pre-existing against the release
binary from before item 14's fix landed, so this isn't a regression from
that work — but it's the identical bug class (assuming a fixed keysym
instead of asking the keymap) in the modifier position rather than the
character position, and the fix is now a short reach: have `resolve_combo`
go through `modifiers::ModifierKeys` (or equivalent) the same way
`type_text` already does, instead of a hard-coded keysym table.
