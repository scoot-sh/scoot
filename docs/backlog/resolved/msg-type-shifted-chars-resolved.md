---
title: "`flexwm msg type` silently drops every shifted character, so an agent cannot type a capital letter (MEDIUM, and squarely against the computer-use goal). \u2014 RESOLVED 2026-09-13 (fix + tests, PR #23)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flexwm msg type` silently drops every shifted character, so an agent cannot type a capital letter (MEDIUM, and squarely against the computer-use goal). — RESOLVED 2026-09-13 (fix + tests, PR #23).

~~`flexwm msg type` silently drops every shifted character, so an agent
cannot type a capital letter (MEDIUM, and squarely against the
computer-use goal).~~ — RESOLVED 2026-09-13 (fix + tests, PR #23).
`needs_shift` is gone. `crates/flexwm/src/compositor/input/modifiers.rs`
answers both halves of the question in one keymap walk: which key carries
the keysym *and* which level it sits at (the old code asked two different
Smithay helpers and only one of them looked past level 0). It then asks
`xkb_keymap_key_get_mods_for_level` which modifier combinations reach that
level and holds the keys for the cheapest one it can actually produce.
*Which* keys those are also comes out of the keymap rather than a
modifier-name-to-keysym table: every keycode is pressed once in a
throwaway `xkb::State` and watched, so AltGr levels work on layouts that
put AltGr somewhere else (checked against `de` and `de(neo)`), a modifier
key with no keysym of its own is still usable, and — the reason the probe
earns its keep — a key that *locks* or *latches* its modifier (Caps Lock,
Num Lock, `ISO_Level3_Latch`) is never pressed. `key_get_mods_for_level`
really does offer Caps Lock as an alternative to Shift for a capital
letter, and pressing it would type one capital and leave the keyboard
shifted for everything typed afterwards. A level only such a key can reach
is an error instead (`Untypable::NoModifiers`), as is a character no key
carries at all (`Untypable::NoKey`) — both loud, where the old behaviour
was silent and wrong. The resolution pass allocates nothing (fixed-size
`Copy` results, and the keymap probe runs at most once per request, only
once a character actually needs a modifier).

**Cost, corrected** (an earlier draft of this entry claimed "one keymap
scan per character instead of two", which the benchmark does not support
and which review caught): the shapes are the same either way. The old
path was one scan (`keycode_for_keysym`) plus one O(1) level-0 lookup;
the new one is one scan (`key_for`) plus one O(1)
`key_get_mods_for_level`. That is exactly why the benchmark shows
lowercase typing unchanged within noise (median 114 ms → 112 ms per
50,000 characters). The real cost is confined to characters that were
previously typed *wrongly*: they now send four key events instead of two
(the modifier's own press and release), which is the whole point, and it
takes the all-shifted median from 108 ms to 188 ms per 50,000.

**Verified** three ways, all red/green (the fix disabled, the tests fail
with exactly the reported symptom; restored, they pass): 10 unit tests
against real `us`/`de`/`de(neo)` keymaps
(`input/modifiers/tests.rs`); 7 live-client tests that decode what a real
`wayland-client` toplevel received through its own `xkb::State`, built
from the keymap fd the compositor sent it (`input/tests.rs` — the
"assert on what the client actually got" test the fix shape below asked
for, in its own harness rather than by extending the layer-shell one);
and a new `scripts/smoke-test.sh` step that types a shell command
containing every broken character class into a real `foot`, redirects it
to a file and diffs it byte for byte. The script itself ran on
`--headless`; the same round trip, run by hand, ran on real `--tty`
hardware in the dev VM (`/home/dev/tty-evidence.sh` there, screenshot
artifact `/home/dev/tty-shift-evidence.png`) — byte-for-byte identical
both times, `od -c` output in PR #23. Deliberately left out of scope and
split out as its own entry below: dead keys, compose sequences, and
characters that are only on a layout other than the active one.

**`flexwm-reviewer` found a blocking bug in the fix itself, fixed in the
same PR (second round).** The first version asked the keymap two
questions in two different xkb *groups*: `ModifierKeys::probe` built a
scratch `xkb::State`, which starts in group 0 and was never pinned, while
`plan` resolved the character's level against the *active* group. Key
actions are per-group exactly as keysyms are, so with `XKB_DEFAULT_LAYOUT=de,de
XKB_DEFAULT_VARIANT=neo, XKB_DEFAULT_OPTIONS=grp:menu_toggle` and the
session toggled into group 1, `flexwm msg type '@'` typed **`#q`** — the
probe had recorded group 0's third-level key (`de(neo)`'s, in the `#`
position), which is an ordinary `#` key in group 1, so the modifier was
never set and `@`'s key fell through to `q`. Silent, and exactly the
failure mode this whole entry exists to close, one level up. `probe` now
takes the layout, pins the state to it (`update_mask`) before every key
it presses, and discards a state that comes back with either the modifier
state or the group changed; `ModifierKeys` carries the layout it answered
for, so `plan`'s per-string cache can't hand a group-0 table to a group-1
character. Every earlier test compiled a single-group keymap, where group
0 *is* the active group, which is why this shipped unnoticed — so the
regression tests are multi-group and assert on what a client decodes.
Reproduced on the dev VM before the fix and re-run after
(`/home/dev/fix-multilayout.sh`), and the real `--tty` round trip above
was re-run against the corrected code rather than carried forward.

The same review round found the identical bug class still live in
`flexwm msg key` — see the entry below for what it did and what replaced
it.

**Where.** `input.rs`'s `needs_shift` decides whether to hold `Shift_L`
around a character, and asks Smithay `xkb.raw_syms_for_key_in_layout(...)
.contains(&keysym)`. That helper is hard-coded to **level 0**
(`input/keyboard/mod.rs:187-189` in the pinned rev:
`key_get_syms_by_level(keycode, layout.0, 0)`), i.e. it returns exactly
the syms that need *no* modifier — so `contains` is false for every
keysym that does need one, and `needs_shift` can only ever answer
"false". `'A'`, `'!'`, `'_'`, `'?'`, `'~'`, `':'` and `'|'` all come out
as their unshifted twin.

**Why it is silent rather than an error.** `keycode_for_keysym`
(`mod.rs:1240-1253`) scans *every* level, so the lookup for `'A'`
succeeds and returns the `a` key; only the shift decision fails. That
asymmetry between the two Smithay calls is the whole bug, and it is also
why three rounds of hardware testing missed it: every test string anyone
happened to type was lowercase.

**Why it matters.** Driving a computer through `flexwm msg type` is a
stated goal of this project, and an agent that cannot produce a capital
letter cannot type a password, a `Dockerfile`, a URL with a query string,
a shell pipeline, or most identifiers in most languages. It is a bigger
practical hole in agent-driven use than anything currently above it in
this backlog.

**Fix shape.** Find the level the keysym actually sits at
(`num_levels_for_key` + `key_get_syms_by_level`, the same walk
`keycode_for_keysym` does) and press the modifiers that level needs —
`xkb_keymap_key_get_mods_for_level` rather than an assumption that level
1 means Shift, since AltGr levels exist on plenty of layouts and `press`
already has a `Modifier` → keysym mapping to reuse. Wants a test that
asserts on what a real client *received* (keysym plus modifier state),
not just that keys arrived: the layer-shell harness is the only one with
a real `wl_keyboard`, and it currently counts events without decoding
them, so it needs extending first.
