---
title: "`flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/ `Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a real modifier off its `_L` key breaks `msg key` combos entirely (LOW, pre-existing). — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/ `Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a real modifier off its `_L` key breaks `msg key` combos entirely (LOW, pre-existing). — RESOLVED

## What it said

`input.rs`'s `resolve_combo` mapped `Modifier::{Ctrl,Shift,Alt,Super}` to
a hard-coded `_L` keysym and then demanded it be reachable at level 0.
Concrete failure: `XKB_DEFAULT_LAYOUT=us,de
XKB_DEFAULT_OPTIONS=grp:lshift_toggle` (or `grp:lctrl_toggle`) removes
`Shift_L`/`Control_L` from the keymap entirely, leaving `Shift_R`/
`Control_R` as the only key carrying that real modifier. `flexwm msg type
"A"` still worked (the probe found `Shift_R`), but `flexwm msg key
shift+a` and `flexwm msg key ctrl+c` were refused with ``no key for
`shift` in this layout``. Fix specified: have `resolve_combo` go through
`modifiers::ModifierKeys` (or equivalent) the same way `type_text` already
does, instead of a hard-coded keysym table.

## Resolution

**Fixed as specified.** `resolve_combo` no longer names a keysym per
modifier; it calls the new `modifiers::modifier_key`, which maps each IPC
`Modifier` to its real modifier (`Shift`→`Shift`, `Ctrl`→`Control`,
`Alt`→`Mod1`, `Super`→`Mod4` — exactly the names Smithay's own
`ModifiersState` reads, so the key found sets precisely what a toolkit
calls that modifier) and returns the probed key depressing it. The
hard-coded `modifier_keysym` table is deleted.

**Level rule, decided explicitly: there is none.** The probe presses keys
and watches the depressed mask without ever consulting keysyms, so a
modifier reached from anywhere but level 0 — or on a key carrying no
nameable keysym at all — is accepted all the same. What disqualifies a key
is behavior, not position: latching/locking instead of holding,
depressing anything else alongside it, or switching the group (which is
why a toggled left Shift is skipped while the right-hand one is found).
Stated in `modifier_key`'s doc comment, not just here.

**Edge cases, each pinned or stated:**
- No key carrying the modifier at all → the same honest `no key for \`x\`
  in this layout` refusal as before (byte-identical string). Pinned by
  `a_modifier_with_no_holdable_key_resolves_to_nothing` against an empty
  probe — no stock xkeyboard-config layout drops a whole modifier (even
  the toggles leave the other hand), so a real-layout pin was not
  expressible; the `None`→refusal string is the pre-existing one, verified
  byte-identical in the fail-first output below.
- Both `_L` + `_R` present → the lowest keycode wins, i.e. the left-hand
  key, the probe's documented first-wins rule. Pinned by
  `with_both_shifts_present_the_probe_holds_the_left_hand_key`.
- Modifier as combo target: `msg key shift` was and stays `unknown key
  \`shift\`` (no such keysym); `msg key Shift_L` still presses the bare
  key on `us`, and is honestly refused where nothing carries it (toggle
  layout). Pinned by `a_bare_modifier_name_is_not_a_pressable_key` and the
  toggle test's `Shift_L` assertion.
- The character position is untouched: `msg key A` still refuses (PR #81's
  pin, re-verified live below), and `shift+A` is refused even though the
  modifier half now resolves — the two halves don't interact. Pinned in
  the toggle test.
- Empty layout/group: unconstructible (a keymap always compiles ≥1 group;
  `with_keymap` answers in the active one), and both failure shapes are
  guarded in code (`MOD_INVALID` → `None`, `keys.get` not `[]` — the
  latter already pinned by
  `a_modifier_past_the_real_ones_is_unproducible_rather_than_a_panic`).

**The test that pinned the hard code is deleted with reason, not silently.**
`modifier_keysyms_are_the_left_variant` asserted the deleted table; that
table is the bug. Its coverage moves to the toggle-layout live tests
(shift/ctrl end to end),
`alt_and_super_resolve_through_the_probe_on_a_plain_layout` (the
Mod1/Mod4 mapping, discriminated by text: a wrongly-held Shift would type
`ISO_Left_Tab`, not `\t`), and
`ipc_modifiers_name_the_modifiers_clients_decode` (the name table
itself). A comment at the deletion site says so.

**Overlap checked: `msg-type-dead-keys-compose.md` does not overlap.**
This change touches only the modifier half of `resolve_combo`. That
entry's AltGr note (`@` on a German layout needs a modifier `key` has no
name for) is unchanged — giving `key` a third-level-modifier name stays
deferred with this pointer.

**Tests** (all fail-first where behavior is concerned):
- `a_combo_with_a_toggled_modifier_presses_the_key_that_still_holds_it`
  (live `State` + real client, seat keymap swapped to
  `us`+`grp:lshift_toggle` via `set_xkb_config`): pre-fix refused with
  `no key for \`shift\` in this layout` (fail-first output below);
  post-fix `shift+a`→`A`, `shift+1`→`!`, `type "A"` unchanged, `shift+A`
  and `Shift_L` refused, refusals sending zero keys.
- `ctrl_c_reaches_the_client_when_the_left_control_is_a_group_toggle`:
  pre-fix `no key for \`ctrl\` in this layout`; post-fix 4 key events.
- `alt_and_super_resolve_through_the_probe_on_a_plain_layout`,
  `a_bare_modifier_name_is_not_a_pressable_key` (pins, pass pre- and
  post-fix).
- Keymap level: `shift/control_is_still_holdable_when_the_left_..._becomes_a_group_toggle`
  (`Shift_L`/`Control_L` `Absent` + probe finds the `_R` key),
  `with_both_shifts_present_the_probe_holds_the_left_hand_key`,
  `ipc_modifiers_name_the_modifiers_clients_decode`,
  `a_modifier_with_no_holdable_key_resolves_to_nothing`.
- Full suite green unmodified otherwise: every pre-existing key/type test
  passes as-is.

**Fail-first record** (dev VM, pre-fix tree: branch
`fix/msg-key-modifier-resolution` at `f6409a6` + tests, fix not applied):
`cargo test -p flexwm -- toggled_modifier group_toggle bare_modifier_name`
→ 3 passed, 2 failed:
`a_combo_with_a_toggled_modifier_presses_the_key_that_still_holds_it`:
`Shift lives on the right-hand key on this layout: "no key for \`shift\`
in this layout"`;
`ctrl_c_reaches_the_client_when_the_left_control_is_a_group_toggle`:
`Control lives on the right-hand key on this layout: "no key for \`ctrl\`
in this layout"`.

**Verified live** (dev VM, post-fix debug binary,
`XKB_DEFAULT_LAYOUT=us XKB_DEFAULT_OPTIONS=grp:lshift_toggle`,
`--headless`): `msg key shift+a` exit 0; `msg key ctrl+c` exit 0;
`shift+h` round-tripped byte-exact through a real `foot` (`Hi` read back
out of the shell, smoke-test style); `msg key A` still refused, exit 1,
with the unmodified-level message. Standard `scripts/smoke-test.sh`
(stock layout): 17 `ok`, no `BUG`/failure.

**Benchmark: none, stated.** `msg key` is per-request IPC, not a
per-frame path. The probe walks the keymap at most once per request
bearing a modifier (shared across the combo, built lazily — a bare-key
request never pays it): a few hundred FFI calls, zero heap, the same
walk `type_text` already pays once per string. No new cost class
introduced.
