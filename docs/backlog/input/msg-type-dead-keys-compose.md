---
title: "`flexwm msg type` still can't produce a character that needs a dead key, a compose sequence, or a layout the session isn't currently on (LOW)."
status: "open"
area: "input"
priority: "low"
blocked: null
---

# `flexwm msg type` still can't produce a character that needs a dead key, a compose sequence, or a layout the session isn't currently on (LOW).

`flexwm msg type` still can't produce a character that needs a dead key,
a compose sequence, or a layout the session isn't currently on (LOW).
The shifted-character fix above covers every character that is *on* the
active layout at some shift level. That is *not* "all of ASCII on any
Latin layout", as an earlier draft of this entry claimed (review caught
it; the numbers below come from
`every_planned_character_decodes_back_to_itself_on_every_latin_layout`
and its sibling, which sweep printable ASCII across fourteen real
layouts and record exactly what each refuses). Coverage is
layout-dependent: `us`, `us(intl)`, `gb`, `de(neo)`, `fr`, `fr(oss)`,
`it` and `pl` produce all 95 printable ASCII characters, while `de` and
`es` refuse `^` and `` ` ``, and `pt`, `se`, `no` and `dk` refuse `~` as
well — those keys carry `dead_circumflex`/`dead_grave`/`dead_tilde`, not
the plain character. `~` is the one that bites in practice: shell paths,
globs and regexes are full of it.

Three things are still out of reach, all of them refused loudly rather
than typed wrong — the first two with ``no key for `X` in this layout``,
the third with its own message (``[X] needs a modifier this layout only
locks or latches``, `input.rs`), since "your layout doesn't have this"
and "your layout has it but only behind Caps Lock" call for different
responses:
- **Dead keys and compose sequences.** `é` on a plain `us` layout is two
  or three keypresses with a state machine in between (`Compose`, `'`,
  `e`), not a key with a level. Driving it needs an `xkb::Compose` table
  and a second resolution path when the single-key lookup fails; the
  payoff is accented Latin text an agent might paste, so this is worth
  doing if that ever comes up, not before.
- **Characters on an inactive layout group.** Resolution uses the
  keymap's active layout only. With `us,de` configured, `ü` is one group
  switch away, and nothing here switches groups — deliberately: the
  session's own layout is user state, and silently changing it to type
  one character (or failing to change it back if the request errors
  partway) is worse than saying no.
- **Levels only a locking or latching modifier reaches.** Refused on
  purpose, see the resolved entry above; a layout that puts a character
  *only* behind Caps Lock would need it pressed and un-pressed around the
  character, and nothing in xkbcommon promises that round-trips cleanly.

Adjacent, and **not** unchanged — an earlier draft of this entry said
`flexwm msg key A` typing `a` was `press`'s documented contract rather
than the same bug, and review measured that it was the same bug, still
live and wider than one name. `key exclam` typed `1`, `key at` typed `2`,
and `asciitilde`, `underscore`, `question`, `colon`, `bar` and
`braceleft` were all off by one level the same way, because
`keycode_for_keysym` returns the lowest keycode carrying a keysym at
*any* level while `press` holds only the modifiers its caller names.
Fixed in the same PR: those names are now refused with a message saying
what to write instead, `modifiers::named_key` resolves names by looking
for a level-0 carrier, and `keycode_for` — the last caller of
`keycode_for_keysym` — is gone. `flexwm msg key shift+1` types `!` and
`shift+a` types `A`, as before.

Note what that refusal does *not* buy: some characters cannot be named as
a combination at all. `@` on a German layout needs AltGr, and `Modifier`
has no name for it (`ctrl`/`shift`/`alt`/`super` only), so `msg type` is
the only way to produce it. Giving `key` a name for the third-level
modifier is a plausible small follow-up if an agent ever needs to chord
with AltGr; nothing needs it today.
